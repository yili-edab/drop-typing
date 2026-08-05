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
//! 组合键支持修饰键家族名（左右任意）或精确修饰键名（区分左右）。
//!
//! Win 键拦截策略：只有「属于 drop-typing 组合」的 Win 键按下/松开才被吞掉，
//! 其余 Win 键事件放行给系统，保证开始菜单、Win+E、Win+R 等系统快捷键不受影响。
//!
//! Windows 低级键盘钩子无需辅助功能权限，但部分杀毒软件可能将全局键盘监听标记为可疑行为。

use std::collections::HashSet;
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
    /// 需要哪些键（家族或精确修饰键）同时按下
    specs: Vec<KeySpec>,
}

/// 从用户配置的 KeySpec 列表构建组合定义。
///
/// Windows 组合只接受修饰键：家族名（Control/Meta/Alt/Shift）或精确修饰键名
/// （ControlLeft/ControlRight/MetaLeft/MetaRight/Alt/AltGr/ShiftLeft/ShiftRight）。
/// 其它精确键（如 KeyA）不适用，会被忽略并打印警告。
fn combo_from_specs(specs: &[KeySpec], down: HotkeyEvent, up: HotkeyEvent) -> ComboDef {
    let mut kept = Vec::new();
    for s in specs {
        match s {
            KeySpec::Family(_) => kept.push(s.clone()),
            KeySpec::Exact(key) if super::is_modifier_key(key) => kept.push(s.clone()),
            KeySpec::Exact(key) => {
                eprintln!(
                    "[drop-typing] 警告：Windows 组合键不支持非修饰键 {key:?}，已忽略。\
                     请使用 Control/Meta/Alt/Shift 家族名或精确修饰键名（含左右）。"
                );
            }
        }
    }
    ComboDef { down, up, specs: kept }
}

/// 检查当前按下的修饰键集合是否满足某个组合定义
fn combo_matches(def: &ComboDef, pressed: &HashSet<Key>) -> bool {
    super::specs_matched_by_pressed(&def.specs, pressed)
}

/// Win 键按下时，判断它是否即将补全某个已配置的 Meta 组合
/// （其它组合修饰键已按住，只差这个 Win 键）。
/// 用于在 keydown 阶段吞掉已知属于 drop-typing 组合的 Win 键，
/// 防止开始菜单在任何触发时机弹出。
fn win_completes_meta_combo(key: &Key, combos: &[ComboDef], pressed: &HashSet<Key>) -> bool {
    combos.iter().any(|c| {
        super::specs_use_meta(&c.specs)
            && c.specs.iter().all(|s| match s {
                KeySpec::Family(ModFamily::Meta) => true,
                KeySpec::Exact(Key::MetaLeft) => {
                    key == &Key::MetaLeft || pressed.contains(&Key::MetaLeft)
                }
                KeySpec::Exact(Key::MetaRight) => {
                    key == &Key::MetaRight || pressed.contains(&Key::MetaRight)
                }
                KeySpec::Family(f) => pressed.iter().any(|k| f.matches(k)),
                KeySpec::Exact(k) => pressed.contains(k),
            })
    })
}

/// 检查鼠标按键是否匹配某个侧键配置
fn mouse_matches(btn: MouseButton, binding: &Option<MouseButton>) -> bool {
    binding.map_or(false, |b| b == btn)
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
                // 当前按下的修饰键集合（区分左右）
                let mut pressed: HashSet<Key> = HashSet::new();
                // 已被 drop-typing 组合使用的 Win 键（松开时吞掉，避免开始菜单）
                let mut win_used: HashSet<Key> = HashSet::new();
                // 当前激活的组合在 combos 中的索引
                let mut active_idx: Option<usize> = None;
                // 鼠标左键双击检测：记录上一次左键按下时间
                let mut last_left_press: Option<std::time::Instant> = None;

                let result = rdev::listen(move |event| {
                    // 组合键录制期间：转发原始事件，不当作业务热键处理
                    if super::capture_active() {
                        super::forward_capture_event(&event);
                        return;
                    }
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

                            if super::is_modifier_key(key) {
                                // Win 键按下且即将补全某个 Meta 组合（如 Alt 已按住）：
                                // 吞掉本次 keydown，避免开始菜单弹出
                                if matches!(key, Key::MetaLeft | Key::MetaRight)
                                    && active_idx.is_none()
                                    && win_completes_meta_combo(key, &combos, &pressed)
                                {
                                    rdev::set_swallow_win_down(true);
                                }
                                pressed.insert(*key);
                                if active_idx.is_none() {
                                    // 检查是否有新的组合形成
                                    if let Some(idx) = combos.iter().position(|c| {
                                        combo_matches(c, &pressed)
                                    }) {
                                        active_idx = Some(idx);
                                        if super::specs_use_meta(&combos[idx].specs) {
                                            // 记录组合用到的 Win 键：松开时吞掉
                                            for k in pressed.iter().filter(|k| {
                                                matches!(k, Key::MetaLeft | Key::MetaRight)
                                            }) {
                                                win_used.insert(*k);
                                            }
                                            // 若本次按下正是 Win 键（Alt 先按住），吞掉 keydown
                                            if matches!(key, Key::MetaLeft | Key::MetaRight) {
                                                rdev::set_swallow_win_down(true);
                                            }
                                        }
                                        let _ = tx.send(combos[idx].down.clone());
                                    }
                                } else if !super::key_matches_any_spec(
                                    key,
                                    &combos[active_idx.unwrap()].specs,
                                ) {
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
                            if super::is_modifier_key(key) {
                                let was_active = active_idx;

                                // 如果释放的键属于当前激活组合 → 结束该组合
                                if let Some(idx) = active_idx {
                                    if super::key_matches_any_spec(key, &combos[idx].specs) {
                                        let _ = tx.send(combos[idx].up.clone());
                                        active_idx = None;
                                    }
                                }

                                // Win 键松开：属于已激活组合则吞掉这次 keyup
                                if matches!(key, Key::MetaLeft | Key::MetaRight)
                                    && win_used.remove(key)
                                {
                                    rdev::set_swallow_win_up(true);
                                }

                                pressed.remove(key);

                                // 如果刚结束了一个组合，检查是否新形成了另一个组合
                                // （「滑键」场景：保持 Win 不放，从 Ctrl 滑到 Alt）
                                if was_active.is_some() && active_idx.is_none() {
                                    if let Some(idx) = combos.iter().position(|c| {
                                        combo_matches(c, &pressed)
                                    }) {
                                        active_idx = Some(idx);
                                        if super::specs_use_meta(&combos[idx].specs) {
                                            for k in pressed.iter().filter(|k| {
                                                matches!(k, Key::MetaLeft | Key::MetaRight)
                                            }) {
                                                win_used.insert(*k);
                                            }
                                        }
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
