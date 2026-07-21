//! macOS 全局热键：rdev 全局事件监听（CGEventTap）。
//!
//! 需要辅助功能（Accessibility）权限，否则 CGEventTap 创建失败、
//! 回调永远收不到事件。用 AXIsProcessTrusted() 检测并在 UI 提示。

use std::sync::mpsc;

use anyhow::Result;
use rdev::{EventType, Key};

use super::{HotkeyEvent, HotkeySource};

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
    fn start(self: Box<Self>, tx: mpsc::Sender<HotkeyEvent>) -> Result<()> {
        std::thread::Builder::new()
            .name("byk-hotkey".into())
            .spawn(move || {
                let result = rdev::listen(move |event| {
                    let ev = match event.event_type {
                        EventType::KeyPress(Key::MetaRight) => Some(HotkeyEvent::TriggerDown),
                        EventType::KeyRelease(Key::MetaRight) => Some(HotkeyEvent::TriggerUp),
                        // 录音期间其它键按下 → 视为组合键用法，作废本次录音
                        EventType::KeyPress(_) => Some(HotkeyEvent::OtherKeyDown),
                        _ => None,
                    };
                    if let Some(ev) = ev {
                        // 接收端退出后发送失败属正常，忽略
                        let _ = tx.send(ev);
                    }
                });
                if let Err(e) = result {
                    eprintln!("[byk] rdev listen error: {e:?}（辅助功能权限未授予？）");
                }
            })?;
        Ok(())
    }

    fn permission_trusted(&self) -> bool {
        accessibility_trusted()
    }
}
