//! Windows 全局热键：rdev 低级键盘钩子（WH_KEYBOARD_LL）。
//!
//! 与 macOS 版本键盘映射不同（macOS 看 macos.rs）：
//! - 空格（Key::Space）→ 输入/提交通道（macOS 的右 ⌘）
//! - 右 Win（Key::MetaRight）→ 修正通道（macOS 的右 ⌥）
//! - 右 Shift（Key::ShiftRight）→ 指令通道（macOS 的右 ⇧）
//!
//! Windows 低级键盘钩子无需辅助功能权限，但部分杀毒软件可能将全局键盘监听标记为可疑行为。

use std::sync::mpsc;

use anyhow::Result;
use rdev::{EventType, Key};

use super::{HotkeyEvent, HotkeySource};

pub struct WindowsHotkey;

impl HotkeySource for WindowsHotkey {
    fn start(self: Box<Self>, tx: mpsc::Sender<HotkeyEvent>) -> Result<()> {
        std::thread::Builder::new()
            .name("drop-typing-hotkey".into())
            .spawn(move || {
                let result = rdev::listen(move |event| {
                    let ev = match event.event_type {
                        EventType::KeyPress(Key::Space) => Some(HotkeyEvent::TriggerDown),
                        EventType::KeyRelease(Key::Space) => Some(HotkeyEvent::TriggerUp),
                        EventType::KeyPress(Key::MetaRight) => Some(HotkeyEvent::RepairDown),
                        EventType::KeyRelease(Key::MetaRight) => Some(HotkeyEvent::RepairUp),
                        EventType::KeyPress(Key::ShiftRight) => Some(HotkeyEvent::CommandDown),
                        EventType::KeyRelease(Key::ShiftRight) => Some(HotkeyEvent::CommandUp),
                        // 录音期间其它键按下 → 视为组合键用法，作废本次录音
                        EventType::KeyPress(_) => Some(HotkeyEvent::OtherKeyDown),
                        _ => None,
                    };
                    if let Some(ev) = ev {
                        let _ = tx.send(ev);
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
