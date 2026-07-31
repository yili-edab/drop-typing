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
pub mod prompts;
pub mod settings;
pub mod staging;
pub mod wakeword;

use staging::{MAX_HEIGHT, WIN_WIDTH};
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
            .inner_size(WIN_WIDTH, MAX_HEIGHT)
            .resizable(false)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible_on_all_workspaces(true)
            // 创建时不抢焦点……
            .focused(false)
            // ……且默认隐藏：按下右 ⌘ 开始录音时才以底部居中显示；
            // 宽度由内容自然撑开（fit-content），高度从一行起、撑到上限后滚动
            .visible(false)
            .build()?;

            // 注册设置相关事件处理器
            settings::register_settings_handlers(app.handle());

            // 发送可用样式列表给暂存条
            settings::emit_styles(app.handle());

            // Windows 平台创建系统托盘图标（macOS 通过 Dock 可见，无需托盘）
            #[cfg(target_os = "windows")]
            {
                use tauri::menu::{MenuBuilder, MenuItemBuilder};
                use tauri::Manager;
                use tauri::tray::TrayIconBuilder;

                let settings_item = MenuItemBuilder::with_id("settings", "设置")
                    .build(app.handle())?;
                let quit_item = MenuItemBuilder::with_id("quit", "退出 drop-typing")
                    .build(app.handle())?;
                let menu = MenuBuilder::new(app.handle())
                    .item(&settings_item)
                    .item(&quit_item)
                    .build()?;
                let _tray = TrayIconBuilder::new()
                    .icon(app.default_window_icon().unwrap().clone())
                    .menu(&menu)
                    .tooltip("drop-typing — 按住空格说话，松开出字")
                    .on_menu_event(|app, event| {
                        if event.id() == "quit" {
                            app.exit(0);
                        }
                        if event.id() == "settings" {
                            if let Some(win) = app.get_webview_window("settings") {
                                let _ = win.show();
                                let _ = win.set_focus();
                            } else {
                                let _ = tauri::WebviewWindowBuilder::new(
                                    app,
                                    "settings",
                                    tauri::WebviewUrl::App("settings.html".into()),
                                )
                                .title("drop-typing 设置")
                                .inner_size(900.0, 600.0)
                                .resizable(true)
                                .decorations(true)
                                .center()
                                .build();
                            }
                        }
                    })
                    .build(app.handle())?;
            }

            pipeline::start(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
