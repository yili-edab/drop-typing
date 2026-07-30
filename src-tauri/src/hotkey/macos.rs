//! macOS 全局热键：rdev 全局事件监听（CGEventTap）。
//!
//! 需要辅助功能（Accessibility）权限，否则 CGEventTap 创建失败、
//! 回调永远收不到事件。用 AXIsProcessTrusted() 检测并在 UI 提示。
//!
//! 所有快捷键均可在 `~/.drop-typing.toml` 的 `[hotkey]` 段中自定义。
//! macOS 端每个通道为一个单键（非组合）。

use std::sync::mpsc;

use anyhow::Result;
use rdev::{EventType, Key};

use super::{Bindings, HotkeyEvent, HotkeySource, KeySpec, MouseButton, MOUSE_DOUBLE_CLICK_MS};

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

/// 辅助功能权限是否已授予
pub fn accessibility_trusted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

pub struct RdevHotkey;

impl HotkeySource for RdevHotkey {
    fn start(
        self: Box<Self>,
        tx: mpsc::Sender<HotkeyEvent>,
        bindings: Bindings,
    ) -> Result<()> {
        std::thread::Builder::new()
            .name("drop-typing-hotkey".into())
            .spawn(move || {
                // 鼠标左键双击检测：记录上一次左键按下时间
                let mut last_left_press: Option<std::time::Instant> = None;
                let result = rdev::listen(move |event| {
                    let ev = match event.event_type {
                        EventType::KeyPress(ref key) => {
                            if matches_any(key, &bindings.trigger) {
                                Some(HotkeyEvent::TriggerDown)
                            } else if matches_any(key, &bindings.repair) {
                                Some(HotkeyEvent::RepairDown)
                            } else if matches_any(key, &bindings.command) {
                                Some(HotkeyEvent::CommandDown)
                            } else if matches_any(key, &bindings.cancel) {
                                Some(HotkeyEvent::CancelDown)
                            } else {
                                // 录音期间其它键按下 → 视为组合键用法，作废本次录音
                                Some(HotkeyEvent::OtherKeyDown)
                            }
                        }
                        EventType::KeyRelease(ref key) => {
                            if matches_any(key, &bindings.trigger) {
                                Some(HotkeyEvent::TriggerUp)
                            } else if matches_any(key, &bindings.repair) {
                                Some(HotkeyEvent::RepairUp)
                            } else if matches_any(key, &bindings.command) {
                                Some(HotkeyEvent::CommandUp)
                            } else {
                                None
                            }
                        }
                        EventType::ButtonPress(rdev::Button::Left) => {
                            let now = std::time::Instant::now();
                            let prev = last_left_press.replace(now);
                            match prev {
                                // 双击判定：触发后重置计时，避免三击连发
                                Some(t)
                                    if now.duration_since(t).as_millis()
                                        < MOUSE_DOUBLE_CLICK_MS as u128 =>
                                {
                                    last_left_press = None;
                                    Some(HotkeyEvent::MouseDoubleClick)
                                }
                                _ => None,
                            }
                        }
                        // ── 鼠标侧键：独立于键盘，绕过修饰键状态机 ──
                        EventType::ButtonPress(rdev::Button::Forward) => {
                            if mouse_matches_button(MouseButton::Forward, &bindings.mouse.trigger)
                            {
                                Some(HotkeyEvent::MouseTriggerDown)
                            } else {
                                None
                            }
                        }
                        EventType::ButtonRelease(rdev::Button::Forward) => {
                            if mouse_matches_button(MouseButton::Forward, &bindings.mouse.trigger)
                            {
                                Some(HotkeyEvent::MouseTriggerUp)
                            } else {
                                None
                            }
                        }
                        EventType::ButtonPress(rdev::Button::Back) => {
                            if mouse_matches_button(MouseButton::Back, &bindings.mouse.repair) {
                                Some(HotkeyEvent::MouseRepairDown)
                            } else {
                                None
                            }
                        }
                        EventType::ButtonRelease(rdev::Button::Back) => {
                            if mouse_matches_button(MouseButton::Back, &bindings.mouse.repair) {
                                Some(HotkeyEvent::MouseRepairUp)
                            } else {
                                None
                            }
                        }
                        _ => None,
                    };
                    if let Some(ev) = ev {
                        let _ = tx.send(ev);
                    }
                });
                if let Err(e) = result {
                    eprintln!("[drop-typing] rdev listen error: {e:?}（辅助功能权限未授予？）");
                }
            })?;
        Ok(())
    }

    fn permission_trusted(&self) -> bool {
        accessibility_trusted()
    }
}

/// 检查某个键是否匹配任意一个规格
fn matches_any(key: &Key, specs: &[KeySpec]) -> bool {
    specs.iter().any(|s| s.matches(key))
}

/// 检查鼠标按键是否匹配某个侧键配置
fn mouse_matches_button(btn: MouseButton, binding: &Option<MouseButton>) -> bool {
    binding.map_or(false, |b| b == btn)
}
