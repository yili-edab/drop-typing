//! Windows 全局热键：rdev 低级键盘钩子（WH_KEYBOARD_LL）。
//!
//! 多键组合方案（与 macOS 单键方案不同）：
//! - 默认 Win+Alt   → 输入/提交通道（长按录音，短按提交）
//! - 默认 Ctrl+Alt  → 语音修复通道
//! - 默认 Win+Shift → 电脑控制（语音指令）通道
//! - 默认 Esc       → 清空暂存条
//!
//! 默认键避开了已知冲突：Ctrl+Win 是微信语音输入的默认键；
//! Shift+Alt 是 Windows 多语言用户的"输入语言切换"默认热键。
//!
//! 所有快捷键均可在 `~/.drop-typing.toml` 的 `[hotkey]` 段中自定义。
//! 每个绑定由一组按键规格组成，所有键必须同时按下才触发。
//!
//! Windows 低级键盘钩子无需辅助功能权限，但部分杀毒软件可能将全局键盘监听标记为可疑行为。

use std::sync::mpsc;

use anyhow::Result;
use rdev::{EventType, Key};

use super::{Bindings, HotkeyEvent, HotkeySource, KeySpec, ModFamily, MouseButton, MOUSE_DOUBLE_CLICK_MS};

// ── 内部组合定义（由 Bindings 转换而来）───────────────────────────

/// 经配置解析后的组合通道定义
struct ComboDef {
    /// 激活时发送哪个 Down/Up 事件
    down: HotkeyEvent,
    up: HotkeyEvent,
    /// 需要哪些修饰键家族同时按下
    ctrl: bool,
    win: bool,
    alt: bool,
    shift: bool,
}

/// 从用户配置的 KeySpec 列表推导每个通道所需的修饰键家族。
///
/// Windows 组合由修饰键家族（Control/Meta/Alt/Shift）构成；
/// 精确键（如 KeyA）在组合中不适用，会被忽略并打印警告。
fn combo_from_specs(
    specs: &[KeySpec],
    down: HotkeyEvent,
    up: HotkeyEvent,
) -> ComboDef {
    let (mut ctrl, mut win, mut alt, mut shift) = (false, false, false, false);
    for s in specs {
        match s {
            KeySpec::Family(ModFamily::Control) => ctrl = true,
            KeySpec::Family(ModFamily::Meta) => win = true,
            KeySpec::Family(ModFamily::Alt) => alt = true,
            KeySpec::Family(ModFamily::Shift) => shift = true,
            KeySpec::Exact(key) => {
                eprintln!(
                    "[drop-typing] 警告：Windows 组合键不支持精确键 {key:?}，已忽略。\
                     请使用 Control/Meta/Alt/Shift 家族名。"
                );
            }
        }
    }
    ComboDef { down, up, ctrl, win, alt, shift }
}

/// 检查修饰键状态是否满足某个组合定义
fn combo_matches(def: &ComboDef, ctrl: bool, win: bool, alt: bool, shift: bool) -> bool {
    // 空组合（所有家族都为 false）不应匹配任何状态
    let has_req = def.ctrl || def.win || def.alt || def.shift;
    has_req
        && (!def.ctrl || ctrl)
        && (!def.win || win)
        && (!def.alt || alt)
        && (!def.shift || shift)
}

/// 检查鼠标按键是否匹配某个侧键配置
fn mouse_matches(btn: MouseButton, binding: &Option<MouseButton>) -> bool {
    binding.map_or(false, |b| b == btn)
}

/// 修饰键家族是否已被某个组合占用（在组合激活期间，按下"不属于"该组的修饰键应 taint）
fn family_in_combo(f: ModFamily, def: &ComboDef) -> bool {
    matches!(
        (f, def.ctrl, def.win, def.alt, def.shift),
        (ModFamily::Control, true, _, _, _)
            | (ModFamily::Meta, _, true, _, _)
            | (ModFamily::Alt, _, _, true, _)
            | (ModFamily::Shift, _, _, _, true)
    )
}

// ── 修饰键工具函数 ────────────────────────────────────────────────

/// 将 rdev Key 映射到修饰键家族（非修饰键返回 None）
fn mod_family(key: &Key) -> Option<ModFamily> {
    match key {
        Key::ControlLeft | Key::ControlRight => Some(ModFamily::Control),
        Key::MetaLeft | Key::MetaRight => Some(ModFamily::Meta),
        Key::Alt | Key::AltGr => Some(ModFamily::Alt),
        Key::ShiftLeft | Key::ShiftRight => Some(ModFamily::Shift),
        _ => None,
    }
}

// ── HotkeySource 实现 ───────────────────────────────────────────

pub struct WindowsHotkey;

impl HotkeySource for WindowsHotkey {
    fn start(
        self: Box<Self>,
        tx: mpsc::Sender<HotkeyEvent>,
        bindings: Bindings,
    ) -> Result<()> {
        // 将用户配置转换为内部组合定义
        let combos = vec![
            combo_from_specs(&bindings.trigger, HotkeyEvent::TriggerDown, HotkeyEvent::TriggerUp),
            combo_from_specs(&bindings.repair, HotkeyEvent::RepairDown, HotkeyEvent::RepairUp),
            combo_from_specs(&bindings.command, HotkeyEvent::CommandDown, HotkeyEvent::CommandUp),
        ];

        // 清空键规格（取消独立于组合，单键触发）
        let cancel_specs = bindings.cancel.clone();

        std::thread::Builder::new()
            .name("drop-typing-hotkey".into())
            .spawn(move || {
                // 修饰键状态（左右合并为同一家族）
                let (mut ctrl, mut win, mut alt, mut shift) =
                    (false, false, false, false);
                // 当前激活的组合在 combos 中的索引
                let mut active_idx: Option<usize> = None;
                // 鼠标左键双击检测：记录上一次左键按下时间
                let mut last_left_press: Option<std::time::Instant> = None;

                let result = rdev::listen(move |event| {
                    match event.event_type {
                        // ── 按键按下 ──
                        EventType::KeyPress(ref key) => {
                            // 清空键：无组合激活时单按即清空
                            if active_idx.is_none()
                                && cancel_specs.iter().any(|s| s.matches(key))
                            {
                                let _ = tx.send(HotkeyEvent::CancelDown);
                                return;
                            }

                            if let Some(fam) = mod_family(key) {
                                // 更新修饰键状态
                                match fam {
                                    ModFamily::Control => ctrl = true,
                                    ModFamily::Meta => win = true,
                                    ModFamily::Alt => alt = true,
                                    ModFamily::Shift => shift = true,
                                }

                                if active_idx.is_none() {
                                    // 检查是否有新的组合形成
                                    if let Some(idx) = combos.iter().position(|c| {
                                        combo_matches(c, ctrl, win, alt, shift)
                                    }) {
                                        active_idx = Some(idx);
                                        let _ = tx.send(combos[idx].down.clone());
                                    }
                                } else if !family_in_combo(fam, &combos[active_idx.unwrap()])
                                {
                                    // 组合激活期间，按下不属于该组合的修饰键 → taint
                                    let _ = tx.send(HotkeyEvent::OtherKeyDown);
                                }
                            } else if active_idx.is_some() {
                                // 非修饰键 + 组合激活中 → taint
                                let _ = tx.send(HotkeyEvent::OtherKeyDown);
                            }
                            // 非修饰键 + 无组合激活 → 忽略
                        }

                        // ── 按键释放 ──
                        EventType::KeyRelease(ref key) => {
                            if let Some(fam) = mod_family(key) {
                                let was_active = active_idx;

                                // 如果释放的键属于当前激活组合 → 结束该组合
                                if let Some(idx) = active_idx {
                                    if family_in_combo(fam, &combos[idx]) {
                                        let _ = tx.send(combos[idx].up.clone());
                                        active_idx = None;
                                    }
                                }

                                // 更新修饰键状态
                                match fam {
                                    ModFamily::Control => ctrl = false,
                                    ModFamily::Meta => win = false,
                                    ModFamily::Alt => alt = false,
                                    ModFamily::Shift => shift = false,
                                }

                                // 如果刚结束了一个组合，检查是否新形成了另一个组合
                                // （「滑键」场景：保持 Win 不放，从 Ctrl 滑到 Alt）
                                if was_active.is_some() && active_idx.is_none() {
                                    if let Some(idx) = combos.iter().position(|c| {
                                        combo_matches(c, ctrl, win, alt, shift)
                                    }) {
                                        active_idx = Some(idx);
                                        let _ = tx.send(combos[idx].down.clone());
                                    }
                                }
                            }
                            // 非修饰键释放 → 忽略
                        }

                        // ── 鼠标左键双击（确认/消除错误）──
                        EventType::ButtonPress(rdev::Button::Left) => {
                            let now = std::time::Instant::now();
                            let prev = last_left_press.replace(now);
                            if let Some(t) = prev {
                                // 双击判定：触发后重置计时，避免三击连发
                                if now.duration_since(t).as_millis()
                                    < MOUSE_DOUBLE_CLICK_MS as u128
                                {
                                    last_left_press = None;
                                    let _ = tx.send(HotkeyEvent::MouseDoubleClick);
                                }
                            }
                        }

                        // ── 鼠标侧键：绕过组合状态机，直接触发 ──
                        EventType::ButtonPress(rdev::Button::Forward) => {
                            if mouse_matches(MouseButton::Forward, &bindings.mouse.trigger) {
                                let _ = tx.send(HotkeyEvent::MouseTriggerDown);
                            }
                        }
                        EventType::ButtonRelease(rdev::Button::Forward) => {
                            if mouse_matches(MouseButton::Forward, &bindings.mouse.trigger) {
                                let _ = tx.send(HotkeyEvent::MouseTriggerUp);
                            }
                        }
                        EventType::ButtonPress(rdev::Button::Back) => {
                            if mouse_matches(MouseButton::Back, &bindings.mouse.repair) {
                                let _ = tx.send(HotkeyEvent::MouseRepairDown);
                            }
                        }
                        EventType::ButtonRelease(rdev::Button::Back) => {
                            if mouse_matches(MouseButton::Back, &bindings.mouse.repair) {
                                let _ = tx.send(HotkeyEvent::MouseRepairUp);
                            }
                        }

                        _ => {} // 其它鼠标事件忽略
                    }
                });
                if let Err(e) = result {
                    eprintln!("[drop-typing] rdev listen error: {e:?}");
                }
            })?;
        Ok(())
    }

    fn permission_trusted(&self) -> bool {
        // Windows 低级键盘钩子无需特殊系统权限
        true
    }
}
