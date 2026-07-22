//! macOS 文字注入：arboard 操作剪贴板 + enigo 模拟 Cmd+V。
//!
//! M1 限制：剪贴板只按纯文本保存/恢复（若用户剪贴板里是图片/文件，
//! 恢复时会丢失，仅恢复不了非文本内容）。后续可接入 NSPasteboard 全量类型快照。
//!
//! 线程约束：macOS 26 起，HIToolbox 的 TSM/TIS 输入法 API 断言必须在主线程
//! 调用（后台线程触发 dispatch_assert_queue → EXC_BREAKPOINT）。enigo 的
//! `Key::Unicode` 需要做键盘布局查询（内部走 TSMGetInputSourceProperty），
//! 因此按键模拟必须调度到主线程执行；剪贴板读写无此限制，仍在调用线程完成。

use std::sync::mpsc;
use std::thread::{self, sleep};
use std::time::Duration;

use anyhow::{Context, Result};
use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};
use tauri::AppHandle;

use super::Injector;

pub struct MacosInjector {
    app: AppHandle,
    /// 主线程 ID（在 `pipeline::start` 中于主线程构造时捕获）
    main_tid: thread::ThreadId,
}

impl MacosInjector {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            main_tid: thread::current().id(),
        }
    }

    /// 在 主线程 上执行 Cmd+V 按键模拟并等待结果。
    fn simulate_paste(&self) -> Result<()> {
        if thread::current().id() == self.main_tid {
            return simulate_paste_on_main();
        }
        let (tx, rx) = mpsc::channel();
        self.app
            .run_on_main_thread(move || {
                let _ = tx.send(simulate_paste_on_main());
            })
            .context("无法将粘贴按键模拟调度到主线程")?;
        rx.recv_timeout(Duration::from_secs(5))
            .context("等待主线程执行粘贴超时")?
    }
}

impl Injector for MacosInjector {
    fn paste_text(&self, text: &str) -> Result<()> {
        let mut clipboard = Clipboard::new().context("无法访问系统剪贴板")?;
        let previous = clipboard.get_text().ok();

        clipboard
            .set_text(text)
            .context("写入剪贴板失败")?;
        // 给剪贴板写入一点生效时间
        sleep(Duration::from_millis(60));

        let paste_result = self.simulate_paste();

        // 等目标 App 取走剪贴板内容后再恢复
        sleep(Duration::from_millis(150));
        if let Some(prev) = previous {
            let _ = clipboard.set_text(prev);
        }

        paste_result
    }
}

fn simulate_paste_on_main() -> Result<()> {
    let mut enigo = Enigo::new(&Settings::default()).context("无法创建键盘模拟器")?;
    enigo.key(Key::Meta, Direction::Press)?;
    enigo.key(Key::Unicode('v'), Direction::Click)?;
    enigo.key(Key::Meta, Direction::Release)?;
    Ok(())
}
