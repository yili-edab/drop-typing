//! macOS 全局热键：rdev 全局事件拦截（CGEventTap Default 模式）。
//!
//! 需要辅助功能（Accessibility）权限，否则 CGEventTap 创建失败、
//! 回调永远收不到事件。用 AXIsProcessTrusted() 检测并在 UI 提示。
//!
//! 所有快捷键均可在 `~/.drop-typing.toml` 的 `[hotkey]` 段中自定义。
//! macOS 端每个通道为一个单键（非组合）。
//!
//! ## grab vs listen
//!
//! 使用 `rdev::grab`（CGEventTapOption::Default）而非 `listen`
//! 是为了拦截已配置的鼠标侧键事件，阻止其原始行为（如浏览器前进/后退导航）。
//! 只有匹配 `[hotkey.mouse]` 配置的侧键才会被消费，其余所有事件全部放行。

use std::sync::mpsc;

use anyhow::Result;
use rdev::{Event, EventType, Key};

use super::{Bindings, HotkeyEvent, HotkeySource, KeySpec, MouseButton};

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
        // 从配置中提取需要拦截的侧键（只有显式配置了的才拦截，未配置则放行）
        let intercept_forward = bindings.mouse.trigger == Some(MouseButton::Forward);
        let intercept_back = bindings.mouse.repair == Some(MouseButton::Back);

        std::thread::Builder::new()
            .name("drop-typing-hotkey".into())
            .spawn(move || {
                let result = rdev::grab(move |event: Event| {
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
                            // 鼠标双击确认已禁用（冲突太多）
                            None
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

                    // ── 事件拦截决策 ──
                    // 只消费已配置的鼠标侧键（阻止浏览器前进/后退等原始行为）；
                    // 所有键盘事件、其它鼠标按键一律放行。
                    match event.event_type {
                        EventType::ButtonPress(rdev::Button::Forward)
                        | EventType::ButtonRelease(rdev::Button::Forward) => {
                            if intercept_forward {
                                None // 消费：禁止浏览器前进
                            } else {
                                Some(event) // 未配置前进键 → 放行
                            }
                        }
                        EventType::ButtonPress(rdev::Button::Back)
                        | EventType::ButtonRelease(rdev::Button::Back) => {
                            if intercept_back {
                                None // 消费：禁止浏览器后退
                            } else {
                                Some(event) // 未配置后退键 → 放行
                            }
                        }
                        _ => Some(event), // 其他所有事件 → 放行
                    }
                });
                if let Err(e) = result {
                    eprintln!(
                        "[drop-typing] rdev grab error: {e:?}（辅助功能权限未授予？）"
                    );
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
