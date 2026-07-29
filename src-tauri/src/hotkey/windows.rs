//! Windows 全局热键：rdev 低级键盘钩子（WH_KEYBOARD_LL）。
//!
//! 多键组合方案（与 macOS 单键方案不同）：
//! - Ctrl+Win  → 输入/提交通道（长按录音，短按提交）
//! - Win+Alt   → 语音控制通道（长按说出指令）
//! - Win+Shift → 语音修复通道（长按说出修正）
//! - Esc        → 清空暂存条并隐藏
//!
//! 实现原理：rdev listen 回调每次只报告单个按键的 press/release，
//! 本模块自行追踪 Ctrl/Win/Alt/Shift 四个修饰键的按下状态，
//! 在状态变化时合成语义 HotkeyEvent（TriggerDown/Up 等）。
//!
//! Windows 低级键盘钩子无需辅助功能权限，但部分杀毒软件可能将全局键盘监听标记为可疑行为。

use std::sync::mpsc;

use anyhow::Result;
use rdev::{EventType, Key};

use super::{HotkeyEvent, HotkeySource};

// ── 组合检测状态机 ──────────────────────────────────────────────

/// 修饰键槽位（左右合并）
#[derive(Clone, Copy, PartialEq, Eq)]
enum ModSlot {
    Ctrl,
    Win,
    Alt,
    Shift,
}

/// 当前激活的组合
#[derive(Clone, Copy, PartialEq, Eq)]
enum ActiveCombo {
    Trigger, // Ctrl+Win
    Repair,  // Win+Alt
    Command, // Win+Shift
}

/// 将 rdev Key 映射到修饰键槽位（非修饰键返回 None）
fn mod_slot(key: &Key) -> Option<ModSlot> {
    match key {
        Key::ControlLeft | Key::ControlRight => Some(ModSlot::Ctrl),
        Key::MetaLeft | Key::MetaRight => Some(ModSlot::Win),
        Key::Alt | Key::AltGr => Some(ModSlot::Alt),
        Key::ShiftLeft | Key::ShiftRight => Some(ModSlot::Shift),
        _ => None,
    }
}

/// 给定槽位是否属于某个组合的成员
fn is_member(slot: ModSlot, combo: ActiveCombo) -> bool {
    matches!(
        (slot, combo),
        (ModSlot::Ctrl, ActiveCombo::Trigger)
            | (ModSlot::Win, ActiveCombo::Trigger)
            | (ModSlot::Win, ActiveCombo::Repair)
            | (ModSlot::Alt, ActiveCombo::Repair)
            | (ModSlot::Win, ActiveCombo::Command)
            | (ModSlot::Shift, ActiveCombo::Command)
    )
}

/// 激活的组合 → Down 事件
fn hotkey_down(combo: ActiveCombo) -> HotkeyEvent {
    match combo {
        ActiveCombo::Trigger => HotkeyEvent::TriggerDown,
        ActiveCombo::Repair => HotkeyEvent::RepairDown,
        ActiveCombo::Command => HotkeyEvent::CommandDown,
    }
}

/// 激活的组合 → Up 事件
fn hotkey_up(combo: ActiveCombo) -> HotkeyEvent {
    match combo {
        ActiveCombo::Trigger => HotkeyEvent::TriggerUp,
        ActiveCombo::Repair => HotkeyEvent::RepairUp,
        ActiveCombo::Command => HotkeyEvent::CommandUp,
    }
}

/// 尝试从当前修饰键状态中识别有效组合
fn detect_combo(ctrl: bool, win: bool, alt: bool, shift: bool) -> Option<ActiveCombo> {
    if ctrl && win {
        Some(ActiveCombo::Trigger) // Ctrl+Win
    } else if win && alt {
        Some(ActiveCombo::Repair) // Win+Alt
    } else if win && shift {
        Some(ActiveCombo::Command) // Win+Shift
    } else {
        None
    }
}

// ── HotkeySource 实现 ───────────────────────────────────────────

pub struct WindowsHotkey;

impl HotkeySource for WindowsHotkey {
    fn start(self: Box<Self>, tx: mpsc::Sender<HotkeyEvent>) -> Result<()> {
        std::thread::Builder::new()
            .name("drop-typing-hotkey".into())
            .spawn(move || {
                // 修饰键状态（左右合并为同一槽位）
                let (mut ctrl, mut win, mut alt, mut shift) =
                    (false, false, false, false);
                let mut active: Option<ActiveCombo> = None;

                let result = rdev::listen(move |event| {
                    match event.event_type {
                        // ── 按键按下 ──
                        EventType::KeyPress(ref key) => {
                            // Esc：无组合激活时 → 清空暂存条
                            if *key == Key::Escape && active.is_none() {
                                let _ = tx.send(HotkeyEvent::CancelDown);
                                return;
                            }

                            if let Some(slot) = mod_slot(key) {
                                // 更新修饰键状态
                                match slot {
                                    ModSlot::Ctrl => ctrl = true,
                                    ModSlot::Win => win = true,
                                    ModSlot::Alt => alt = true,
                                    ModSlot::Shift => shift = true,
                                }

                                if active.is_none() {
                                    // 检查是否新形成有效组合
                                    if let Some(combo) =
                                        detect_combo(ctrl, win, alt, shift)
                                    {
                                        active = Some(combo);
                                        let _ = tx.send(hotkey_down(combo));
                                    }
                                } else if !is_member(slot, active.unwrap()) {
                                    // 组合激活期间按下不属于该组合的修饰键 → taint
                                    let _ = tx.send(HotkeyEvent::OtherKeyDown);
                                }
                            } else if active.is_some() {
                                // 非修饰键 + 组合激活中 → taint
                                let _ = tx.send(HotkeyEvent::OtherKeyDown);
                            }
                            // 非修饰键 + 无组合激活 → 忽略
                        }

                        // ── 按键释放 ──
                        EventType::KeyRelease(ref key) => {
                            if let Some(slot) = mod_slot(key) {
                                let was_active = active;

                                // 如果释放的键属于当前激活组合 → 结束该组合
                                if let Some(combo) = active {
                                    if is_member(slot, combo) {
                                        let _ = tx.send(hotkey_up(combo));
                                        active = None;
                                    }
                                }

                                // 更新修饰键状态
                                match slot {
                                    ModSlot::Ctrl => ctrl = false,
                                    ModSlot::Win => win = false,
                                    ModSlot::Alt => alt = false,
                                    ModSlot::Shift => shift = false,
                                }

                                // 如果刚结束了一个组合，检查是否新形成了另一个组合
                                // （「滑键」场景：保持 Win 不放，从 Ctrl 滑到 Alt）
                                if was_active.is_some() && active.is_none() {
                                    if let Some(new_combo) =
                                        detect_combo(ctrl, win, alt, shift)
                                    {
                                        active = Some(new_combo);
                                        let _ = tx.send(hotkey_down(new_combo));
                                    }
                                }
                            }
                            // 非修饰键释放 → 忽略
                        }

                        _ => {} // 鼠标事件忽略
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
