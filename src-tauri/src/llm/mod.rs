//! LLM 清洗层抽象（M2）。
//!
//! 与 `asr/` 同构：一套 trait + 每种协议一个适配器，`[llm].protocol` 决定适配器：
//! - `openai-chat`（默认）：OpenAI Chat Completions 兼容协议（PRD 6.4 统一约定，
//!   一套实现兼容 DeepSeek / OpenAI / Qwen / Ollama）
//! - `anthropic-messages`：Anthropic Messages API 兼容协议（如百炼 /apps/anthropic 端点）
//!
//! 未配置 `[llm]` 或缺 api_key 时 `cleaner_from_config` 返回 None，清洗层关闭，
//! ASR 直出（PRD 第 10 节"成本控制"要求的关闭清洗选项）。

pub mod anthropic;
pub mod openai;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::config::Config;

/// 语音修正专用 system prompt。输入原文 + 修正说明，输出修正后全文。
fn repair_system_prompt() -> &'static str {
    "你是语音转写文本的修正助手。你会收到两部分：\
     【原文】是暂存条中已有的文本，【修正说明】是用户对原文的修改指令。\
     根据修正说明修改原文，只输出修正后的全文。\
     不要输出任何解释、引号或前后缀，不要输出修正说明本身。"
}

/// 文本清洗接口。
#[async_trait]
pub trait TextCleaner: Send + Sync {
    /// 口语清洗（输入通道）：去掉口水话、修正标点、中英空格。
    /// `system_prompt` 由调用方根据用户配置构建后传入。
    async fn clean(&self, text: &str, system_prompt: &str) -> Result<String>;

    /// 语音修正（修正通道）：根据修正说明修改原文，输出修正后全文。
    /// `original` 是暂存条当前全文，`instruction` 是用户说出的修正指令（ASR 转写结果）。
    async fn repair(&self, original: &str, instruction: &str) -> Result<String>;
}

/// 根据配置构造清洗器。未配置 `[llm]`、缺 api_key 或协议未知时返回 None。
///
/// dispatch 只看 `protocol`（适配器选择）；`provider` 是厂商名，不参与分发。
pub fn cleaner_from_config(cfg: &Config) -> Option<Arc<dyn TextCleaner>> {
    let key = cfg.llm_api_key()?;
    let model = cfg.llm.model.clone();
    match cfg.llm_protocol().as_str() {
        "openai-chat" => Some(Arc::new(openai::OpenAiCleaner::new(
            key,
            model,
            cfg.llm.base_url.as_deref(),
        ))),
        "anthropic-messages" => match anthropic::AnthropicCleaner::new(
            key,
            model,
            cfg.llm.base_url.as_deref(),
        ) {
            Ok(c) => Some(Arc::new(c)),
            Err(e) => {
                eprintln!("[drop-typing] llm init failed: {e}");
                None
            }
        },
        other => {
            eprintln!("[drop-typing] unknown llm protocol: {other}");
            None
        }
    }
}

/// 清洗结果后处理。始终应用中英混排空格正则兜底。
pub(crate) fn post_process(text: &str) -> String {
    pangu_spacing(text)
}

/// 中英混排空格兜底（pangu 风格）：CJK 字符与 ASCII 字母/数字相邻时插入空格。
/// LLM 为主、本函数做正则后处理兜底（PRD 4.1）。
pub(crate) fn pangu_spacing(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 8);
    for (i, &c) in chars.iter().enumerate() {
        if i > 0 {
            let prev = chars[i - 1];
            if (is_cjk(prev) && is_ascii_alnum(c)) || (is_ascii_alnum(prev) && is_cjk(c)) {
                out.push(' ');
            }
        }
        out.push(c);
    }
    out
}

fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{4e00}'..='\u{9fff}'   // CJK 统一表意文字
        | '\u{3400}'..='\u{4dbf}' // 扩展 A
        | '\u{f900}'..='\u{faff}' // 兼容表意文字
        | '\u{3000}'..='\u{303f}' // CJK 标点
        | '\u{ff00}'..='\u{ffef}' // 全角字符
    )
}

fn is_ascii_alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}
