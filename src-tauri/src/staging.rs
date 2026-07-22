//! 暂存条状态管理。
//!
//! 文本归属 Rust 侧（单一事实来源），前端只负责渲染事件。
//! M2/M3 的清洗、修正通道都操作这里的状态。

use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter};

#[derive(Clone)]
pub struct Staging {
    app: AppHandle,
    text: Arc<Mutex<String>>,
    last_error: Arc<Mutex<Option<String>>>,
}

impl Staging {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            text: Arc::new(Mutex::new(String::new())),
            last_error: Arc::new(Mutex::new(None)),
        }
    }

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

    /// 实时识别的中间结果（累积全文，前端弱化样式展示）
    pub fn partial(&self, text: &str) {
        let _ = self
            .app
            .emit("drop-typing://partial", serde_json::json!({ "text": text }));
    }

    /// 异常提示（PRD 3.3：整条黄底红字）
    pub fn error(&self, message: &str) {
        *self.last_error.lock().unwrap() = Some(message.to_string());
        let _ = self
            .app
            .emit("drop-typing://error", serde_json::json!({ "message": message }));
    }

    /// 提交成功反馈
    pub fn committed(&self) {
        let _ = self.app.emit("drop-typing://committed", ());
    }
}
