//! ASR Provider 抽象层。
//!
//! PRD 6.4：每种协议一个适配器（各家 ASR API 格式差异大，不做单一协议假设）。
//! 配置中 `provider` 表厂商（文档/分组用），`protocol` 决定用哪个适配器。
//!
//! 两种形态：
//! - 批量（`AsrProvider`）：录完整个 WAV 一次性转写（M1 的 HTTP 方案，保留作备选）
//! - 实时（`RealtimeAsrProvider`）：WebSocket 流式，边录边传边出字（当前默认）

pub mod bailian;
pub mod bailian_realtime;

use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

use crate::config::Config;

/// 批量转写接口（一次性）。
#[async_trait]
pub trait AsrProvider: Send + Sync {
    /// `wav_bytes`：16kHz 单声道 WAV；`context`：可选上下文偏置（PRD 6.4）
    async fn transcribe(&self, wav_bytes: &[u8], context: Option<&str>) -> Result<String>;
}

/// 实时识别会话（一次"按住说话"对应一个会话）。
///
/// 音频格式：PCM s16le / 16kHz / 单声道。
pub trait RealtimeSession: Send + Sync {
    /// 非阻塞发送一段 PCM chunk
    fn send_audio(&self, pcm: &[u8]) -> Result<()>;
    /// 标记输入结束并阻塞等待最终全文（内部有超时保护）
    fn finish(&self) -> Result<String>;
}

/// 实时识别 Provider。
pub trait RealtimeAsrProvider: Send + Sync {
    /// 建立会话（连接 WS + 下发任务），快速失败。
    /// `partial_tx` 用于推送中间结果（累积全文，前端直接展示）。
    fn start_session(&self, partial_tx: mpsc::Sender<String>)
        -> Result<Box<dyn RealtimeSession>>;
}

/// 统一后端：pipeline 只面向这个枚举
pub enum AsrBackend {
    Batch(Arc<dyn AsrProvider>),
    Realtime(Arc<dyn RealtimeAsrProvider>),
}

/// 根据配置构造 ASR 后端。Key 缺失或协议未知时返回 None。
///
/// dispatch 只看 `protocol`（适配器选择）；`provider` 是厂商名，不参与分发。
pub fn backend_from_config(cfg: &Config) -> Option<AsrBackend> {
    let key = cfg.asr_api_key()?;
    let model = cfg.asr_model_name();
    match cfg.asr_protocol().as_str() {
        "dashscope-http" => Some(AsrBackend::Batch(Arc::new(bailian::BailianAsr::new(
            key, model,
        )))),
        "dashscope-realtime" => {
            match bailian_realtime::BailianRealtimeAsr::new(
                key,
                model,
                cfg.asr.base_url.as_deref(),
            ) {
                Ok(p) => Some(AsrBackend::Realtime(Arc::new(p))),
                Err(e) => {
                    eprintln!("[byk] realtime asr init failed: {e}");
                    None
                }
            }
        }
        other => {
            eprintln!("[byk] unknown asr protocol: {other}");
            None
        }
    }
}

/// finish() 等待最终结果的超时
pub(crate) const FINISH_TIMEOUT: Duration = Duration::from_secs(15);
