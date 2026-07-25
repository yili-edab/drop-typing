//! 暂存条状态管理。
//!
//! 文本归属 Rust 侧（单一事实来源），前端只负责渲染事件。
//!
//! 定位：永远固定屏幕底部居中。高度从一行起，随内容增长到上限后滚动；
//! 宽度由内容自然撑开（前端 fit-content），上限为窗口宽度。

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, Listener, Manager, PhysicalPosition};

/// 暂存条窗口宽度（逻辑像素）
pub(crate) const WIN_WIDTH: f64 = 640.0;
/// 底部居中时距屏幕底部的偏移（逻辑像素）
pub(crate) const BOTTOM_OFFSET: f64 = 110.0;
/// 窗口高度夹取范围（一行约 60px，最多 ~92px / ~4 行）
pub(crate) const MIN_HEIGHT: f64 = 60.0;
pub(crate) const MAX_HEIGHT: f64 = 272.0;

#[derive(Clone)]
pub struct Staging {
    app: AppHandle,
    text: Arc<Mutex<String>>,
    last_error: Arc<Mutex<Option<String>>>,
}

impl Staging {
    pub fn new(app: AppHandle) -> Self {
        let staging = Self {
            app: app.clone(),
            text: Arc::new(Mutex::new(String::new())),
            last_error: Arc::new(Mutex::new(None)),
        };

        // 前端测量内容高度后通过 "drop-typing://resize" 请求调整窗口高度。
        // 高度从一行起，上限受 CSS #bar max-height 约束；超出后滚动。
        let st = staging.clone();
        app.listen("drop-typing://resize", move |event| {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                if let Some(h) = v.get("height").and_then(|h| h.as_f64()) {
                    st.apply_resize(h);
                }
            }
        });

        // 前端关闭按钮：清空内容与异常态
        let st = staging.clone();
        app.listen("drop-typing://close", move |_| {
            st.dismiss();
        });

        staging
    }

    fn window(&self) -> Option<tauri::WebviewWindow> {
        self.app.get_webview_window("staging")
    }

    fn apply_resize(&self, height: f64) {
        let Some(win) = self.window() else { return };
        let h = height.clamp(MIN_HEIGHT, MAX_HEIGHT);
        // 保持底边不动向上长：按窗口自身 scale 计算新旧物理高度差，移动顶边
        if let (Ok(size), Ok(pos)) = (win.outer_size(), win.outer_position()) {
            let scale = size.width as f64 / WIN_WIDTH;
            let dy = h * scale - size.height as f64;
            let _ = win.set_position(PhysicalPosition::new(pos.x, pos.y - dy as i32));
        }
        let _ = win.set_size(tauri::Size::Logical(tauri::LogicalSize::new(WIN_WIDTH, h)));
    }

    // ---------- 显隐与定位 ----------

    /// 显示暂存条：永远屏幕底部居中。
    /// 按下右 ⌘ 开始录音时才显示；错误（error()）也会顺带显示。
    pub fn show(&self) {
        self.show_at_bottom()
    }

    /// 屏幕底部居中定位
    fn show_at_bottom(&self) {
        let Some(win) = self.window() else { return };
        if let Ok(Some(monitor)) = win.current_monitor() {
            let scale = monitor.scale_factor();
            let screen = monitor.size();
            let x = (screen.width as f64 - WIN_WIDTH * scale) / 2.0;
            let y = screen.height as f64 - (BOTTOM_OFFSET + MIN_HEIGHT) * scale;
            let _ = win.set_position(PhysicalPosition::new(
                x.max(0.0) as i32,
                y.max(0.0) as i32,
            ));
        }
        let _ = win.show();
    }

    pub fn hide(&self) {
        if let Some(win) = self.window() {
            let _ = win.hide();
        }
    }

    /// 关闭按钮：清空内容、清除异常态（窗口由前端直接隐藏）
    fn dismiss(&self) {
        *self.text.lock().unwrap() = String::new();
        *self.last_error.lock().unwrap() = None;
        self.emit_text();
        let _ = self.app.emit("drop-typing://status", serde_json::json!({ "status": "" }));
        let _ = self.app.emit("drop-typing://partial", serde_json::json!({ "text": "" }));
        let _ = self.app.emit("drop-typing://busy", serde_json::json!({ "busy": false }));
        let _ = self.app.emit("drop-typing://recording", serde_json::json!({ "recording": false }));
        let _ = self.app.emit("drop-typing://repair-note", serde_json::json!({ "text": "" }));
        let _ = self.app.emit("drop-typing://command-clear", ());
    }

    // ---------- 状态与文本 ----------

    /// 前端加载完成后请求重发当前状态（启动早期发出的事件会被错过）
    pub fn republish(&self) {
        self.emit_text();
        let err = self.last_error.lock().unwrap().clone();
        if let Some(e) = err {
            let _ = self
                .app
                .emit("drop-typing://error", serde_json::json!({ "message": e }));
        }
    }

    fn emit_text(&self) {
        let text = self.text.lock().unwrap().clone();
        let _ = self.app.emit("drop-typing://staging", serde_json::json!({ "text": text }));
    }

    /// 追加一段转写结果（PRD 3.3：多次录音追加在已有内容之后）
    pub fn append(&self, segment: &str) {
        {
            let mut t = self.text.lock().unwrap();
            t.push_str(segment);
        }
        *self.last_error.lock().unwrap() = None; // 成功追加即清除异常态
        self.emit_text();
    }

    /// 当前全文（修正通道 / ASR 上下文偏置会用）
    pub fn text(&self) -> String {
        self.text.lock().unwrap().clone()
    }

    /// 取出并清空（提交时使用）
    pub fn take(&self) -> String {
        let text = {
            let mut t = self.text.lock().unwrap();
            std::mem::take(&mut *t)
        };
        self.emit_text();
        text
    }

    /// 直接设置全文（提交失败时回滚内容用；M3 修正通道整体替换也会用）
    pub fn set_text(&self, text: &str) {
        {
            let mut t = self.text.lock().unwrap();
            *t = text.to_string();
        }
        self.emit_text();
    }

    /// 整体替换暂存条内容（M2 修正通道）。清空原文 → 写入新文本 → 清除异常态。
    pub fn replace(&self, text: &str) {
        {
            let mut t = self.text.lock().unwrap();
            *t = text.to_string();
        }
        *self.last_error.lock().unwrap() = None;
        self.emit_text();
    }

    /// 设置修正通道的修复意见展示（独立于暂存条正文）。
    /// 传空串即隐藏。
    pub fn set_repair_note(&self, text: &str) {
        let _ = self
            .app
            .emit("drop-typing://repair-note", serde_json::json!({ "text": text }));
    }

    /// 展示识别出的按键指令（M4）：大字显示 + 右侧秒级倒计时
    pub fn show_command(&self, display: &str, seconds: u64) {
        let _ = self.app.emit(
            "drop-typing://command",
            serde_json::json!({ "text": display, "seconds": seconds }),
        );
    }

    /// 指令倒计时每秒更新
    pub fn command_tick(&self, seconds: u64) {
        let _ = self
            .app
            .emit("drop-typing://command-tick", serde_json::json!({ "seconds": seconds }));
    }

    /// 清除指令展示（执行完毕 / 新录音开始 / 关闭按钮）
    pub fn clear_command(&self) {
        let _ = self.app.emit("drop-typing://command-clear", ());
    }

    /// 录音状态（前端波形动画）
    pub fn set_recording(&self, recording: bool) {
        let _ = self
            .app
            .emit("drop-typing://recording", serde_json::json!({ "recording": recording }));
    }

    /// 转写中状态
    pub fn set_busy(&self, busy: bool) {
        let _ = self
            .app
            .emit("drop-typing://busy", serde_json::json!({ "busy": busy }));
    }

    /// 右侧状态徽章（倾听中 / 识别中 / 润色中；空串为隐藏）
    pub fn set_status(&self, status: &str) {
        let _ = self
            .app
            .emit("drop-typing://status", serde_json::json!({ "status": status }));
    }

    /// 实时识别的中间结果（累积全文，前端弱化样式展示）。
    /// 传空串可清掉上一轮的残留中间结果。
    pub fn partial(&self, text: &str) {
        let _ = self
            .app
            .emit("drop-typing://partial", serde_json::json!({ "text": text }));
    }

    /// 清除异常态（开始新一次录音时调用）
    pub fn clear_error(&self) {
        *self.last_error.lock().unwrap() = None;
        self.emit_text();
    }

    /// 异常提示（PRD 3.3：整条黄底红字）。错误必须可见：
    /// 窗口隐藏时先显示出来。
    pub fn error(&self, message: &str) {
        *self.last_error.lock().unwrap() = Some(message.to_string());
        let _ = self
            .app
            .emit("drop-typing://error", serde_json::json!({ "message": message }));
        if let Some(win) = self.window() {
            if !win.is_visible().unwrap_or(false) {
                self.show();
            }
        }
    }

    /// 提交成功反馈
    pub fn committed(&self) {
        let _ = self.app.emit("drop-typing://committed", ());
    }
}
