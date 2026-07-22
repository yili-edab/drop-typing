//! drop-typing — M1
//!
//! 模块边界（为 M2 LLM 清洗 / M3 修正通道 / M4 指令通道预留）：
//! - `asr/`     ASR Provider 抽象（trait + 每家服务商一个适配器）
//! - `audio/`   录音（cpal，16kHz 单声道 WAV）
//! - `hotkey/`  全局热键（平台相关，trait 抽象；macOS 用 rdev）
//! - `inject/`  文字注入（平台相关，trait 抽象；macOS 用剪贴板 + 模拟 Cmd+V）
//! - `staging`  暂存条状态管理（文本归属 Rust 侧，前端只负责渲染）
//! - `pipeline` 编排：热键事件 → 录音 → ASR → 暂存条 → 提交
//! - `config`   用户级配置（config.toml）

pub mod asr;
pub mod audio;
pub mod config;
pub mod hotkey;
pub mod inject;
pub mod pipeline;
pub mod staging;

use tauri::{Listener, WebviewUrl, WebviewWindowBuilder};

/// 暂存条窗口宽度（逻辑像素）
const WIN_WIDTH: f64 = 640.0;
/// 距屏幕底部的偏移（逻辑像素）
const BOTTOM_OFFSET: f64 = 110.0;
/// 窗口高度夹取范围
const MIN_HEIGHT: f64 = 48.0;
const MAX_HEIGHT: f64 = 260.0;

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
            .build()?;

            // ……且 M1 暂存条为只读展示：直接忽略鼠标事件，
            // 窗口永远不会成为 key window，等效于 non-activating panel。
            // M3 加入手动编辑时需移除该行并改用 NSPanel 方案。
            let _ = window.set_ignore_cursor_events(true);

            // 定位：主屏底部居中
            if let Ok(Some(monitor)) = window.current_monitor() {
                let scale = monitor.scale_factor();
                let screen = monitor.size();
                let x = (screen.width as f64 - WIN_WIDTH * scale) / 2.0;
                let y = screen.height as f64 - (BOTTOM_OFFSET + MIN_HEIGHT) * scale;
                let _ = window.set_position(tauri::PhysicalPosition::new(
                    x.max(0.0) as i32,
                    y.max(0.0) as i32,
                ));
            }
            let _ = window.show();

            // 前端测量内容高度后通过 "drop-typing://resize" 请求调整窗口高度（多行自适应）
            let win = window.clone();
            app.listen("drop-typing://resize", move |event| {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                    if let Some(h) = v.get("height").and_then(|h| h.as_f64()) {
                        let h = h.clamp(MIN_HEIGHT, MAX_HEIGHT);
                        let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(
                            WIN_WIDTH, h,
                        )));
                    }
                }
            });

            pipeline::start(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
