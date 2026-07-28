//! Windows 文字注入：arboard 操作剪贴板 + enigo 模拟 Ctrl+V。
//!
//! 与 macOS 版本对照：
//! - 粘贴用 Ctrl+V（macOS 用 Cmd+V）
//! - Modifier::Command 映射为 Ctrl（Windows 快捷键惯用 Ctrl 而非 Win）
//! - 无需调度到主线程（macOS 26 的 TSM 约束是 macOS 专属，Windows 无此限制）
//!
//! 已知限制（与 macOS 版一致）：剪贴板只按纯文本保存/恢复，
//! 非文本内容（图片/文件）恢复时会丢失。

use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

use super::Injector;
use crate::command::{KeyCombo, Modifier};

pub struct WindowsInjector;

impl WindowsInjector {
    pub fn new(_app: tauri::AppHandle) -> Self {
        // Windows 不需要像 macOS 那样调度到主线程，AppHandle 仅用于签名一致
        Self
    }
}

impl Injector for WindowsInjector {
    fn paste_text(&self, text: &str) -> Result<()> {
        let mut clipboard = Clipboard::new().context("无法访问系统剪贴板")?;
        let previous = clipboard.get_text().ok();

        clipboard
            .set_text(text)
            .context("写入剪贴板失败")?;
        // 给剪贴板写入一点生效时间
        thread::sleep(Duration::from_millis(60));

        let paste_result = simulate_combo_impl(&KeyCombo {
            modifiers: vec![Modifier::Control],
            key: "V".to_string(),
        });

        // 等目标 App 取走剪贴板内容后再恢复
        thread::sleep(Duration::from_millis(150));
        if let Some(prev) = previous {
            let _ = clipboard.set_text(prev);
        }

        paste_result
    }

    fn simulate_combo(&self, combo: &KeyCombo) -> Result<()> {
        simulate_combo_impl(combo)
    }
}

/// 键名（command.rs 的规范化形式）→ enigo 键位。
/// 与 macOS 版完全一致——enigo 的 Key 枚举是跨平台的。
fn enigo_key(name: &str) -> Result<Key> {
    if name.len() == 1 {
        let c = name.chars().next().unwrap();
        if c.is_ascii_alphanumeric() {
            return Ok(Key::Unicode(c.to_ascii_lowercase()));
        }
    }
    match name {
        "ENTER" => Ok(Key::Return),
        "SPACE" => Ok(Key::Space),
        "TAB" => Ok(Key::Tab),
        "ESC" => Ok(Key::Escape),
        "DELETE" => Ok(Key::Backspace),
        "UP" => Ok(Key::UpArrow),
        "DOWN" => Ok(Key::DownArrow),
        "LEFT" => Ok(Key::LeftArrow),
        "RIGHT" => Ok(Key::RightArrow),
        "F1" => Ok(Key::F1),
        "F2" => Ok(Key::F2),
        "F3" => Ok(Key::F3),
        "F4" => Ok(Key::F4),
        "F5" => Ok(Key::F5),
        "F6" => Ok(Key::F6),
        "F7" => Ok(Key::F7),
        "F8" => Ok(Key::F8),
        "F9" => Ok(Key::F9),
        "F10" => Ok(Key::F10),
        "F11" => Ok(Key::F11),
        "F12" => Ok(Key::F12),
        _ => anyhow::bail!("不支持的键名：{name}"),
    }
}

/// 修饰键映射。
///
/// Windows 语义：
/// - Modifier::Command → Ctrl（Windows 没有 Cmd 键，大多数快捷键用 Ctrl 替代）
/// - Modifier::Control → Ctrl
/// - Modifier::Option  → Alt
/// - Modifier::Shift   → Shift
///
/// 注意：Command 和 Control 都映射为 Ctrl，这意味着语音指令"命令加C"和
/// "控制加C"在 Windows 上效果相同（都是 Ctrl+C）。后续可考虑在 command.rs
/// 中增加 Modifier::Win 变体以支持 Win 键组合。
fn modifier_key(m: &Modifier) -> Key {
    match m {
        Modifier::Command => Key::Control,
        Modifier::Control => Key::Control,
        Modifier::Shift => Key::Shift,
        Modifier::Option => Key::Alt,
    }
}

/// 按下全部修饰键 → Click 目标键 → 逆序松开修饰键
fn simulate_combo_impl(combo: &KeyCombo) -> Result<()> {
    let mut enigo = Enigo::new(&Settings::default()).context("无法创建键盘模拟器")?;
    for m in &combo.modifiers {
        enigo.key(modifier_key(m), Direction::Press)?;
    }
    enigo.key(enigo_key(&combo.key)?, Direction::Click)?;
    for m in combo.modifiers.iter().rev() {
        enigo.key(modifier_key(m), Direction::Release)?;
    }
    Ok(())
}
