//! macOS 文字注入：arboard 操作剪贴板 + enigo 模拟 Cmd+V。
//!
//! M1 限制：剪贴板只按纯文本保存/恢复（若用户剪贴板里是图片/文件，
//! 恢复时会丢失，仅恢复不了非文本内容）。后续可接入 NSPasteboard 全量类型快照。

use std::thread::sleep;
use std::time::Duration;

use anyhow::{Context, Result};
use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

use super::Injector;

pub struct MacosInjector;

impl Injector for MacosInjector {
    fn paste_text(&self, text: &str) -> Result<()> {
        let mut clipboard = Clipboard::new().context("无法访问系统剪贴板")?;
        let previous = clipboard.get_text().ok();

        clipboard
            .set_text(text)
            .context("写入剪贴板失败")?;
        // 给剪贴板写入一点生效时间
        sleep(Duration::from_millis(60));

        let paste_result = simulate_paste();

        // 等目标 App 取走剪贴板内容后再恢复
        sleep(Duration::from_millis(150));
        if let Some(prev) = previous {
            let _ = clipboard.set_text(prev);
        }

        paste_result
    }
}

fn simulate_paste() -> Result<()> {
    let mut enigo = Enigo::new(&Settings::default()).context("无法创建键盘模拟器")?;
    enigo.key(Key::Meta, Direction::Press)?;
    enigo.key(Key::Unicode('v'), Direction::Click)?;
    enigo.key(Key::Meta, Direction::Release)?;
    Ok(())
}
