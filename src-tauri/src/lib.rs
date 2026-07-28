//! drop-typing — M2
//!
//! 模块边界（为 M3 修正通道 / M4 指令通道预留）：
//! - `asr/`     ASR Provider 抽象（trait + 每家服务商一个适配器）
//! - `llm/`     LLM 清洗层抽象（trait + 每种协议一个适配器，M2）
//! - `audio/`   录音（cpal，16kHz 单声道 WAV）
//! - `hotkey/`  全局热键（平台相关，trait 抽象；macOS 用 rdev）
//! - `inject/`  文字注入（平台相关，trait 抽象；macOS 用剪贴板 + 模拟 Cmd+V）
//! - `caret`    光标屏幕位置查询（macOS AX API，预留；当前未使用）
//! - `staging`  暂存条状态与窗口显隐（文本归属 Rust 侧，前端只负责渲染；永远底部居中）
//! - `pipeline` 编排：热键事件 → 录音 → ASR → 清洗 → 暂存条 → 提交
//! - `config`   用户级配置（config.toml）

pub mod asr;
pub mod audio;
#[cfg(target_os = "macos")]
pub mod caret;
pub mod command;
pub mod config;
pub mod hotkey;
pub mod inject;
pub mod llm;
pub mod pipeline;
pub mod staging;

use staging::{MIN_HEIGHT, WIN_WIDTH};
use tauri::{WebviewUrl, WebviewWindowBuilder};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let window = WebviewWindowBuilder::new(
                app,
                "staging",
                WebviewUrl::App("index.html".into()),
            )
            .title("drop-typing")
            .inner_size(WIN_WIDTH, MIN_HEIGHT)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible_on_all_workspaces(true)
            // 创建时不抢焦点……
            .focused(false)
            // ……且默认隐藏：按下右 ⌘ 开始录音时才以底部居中显示；
            // 宽度由内容自然撑开（fit-content），高度从一行起、撑到上限后滚动
            .visible(false)
            .build()?;

            // ……且 M1 暂存条为只读展示：直接忽略鼠标事件，
            // 窗口永远不会成为 key window，等效于 non-activating panel。
            // M3 加入手动编辑时需移除该行并改用 NSPanel 方案。
            let _ = window.set_ignore_cursor_events(true);

            pipeline::start(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
