//! ASR Provider 抽象层。
//!
//! PRD 6.4：每家服务商一个适配器（各家 ASR API 格式差异大，不做单一协议假设）。
//! M2/M3 如需切换或新增服务商，在此目录新增适配器并在 `provider_from_config` 注册。

pub mod bailian;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use crate::config::Config;

/// 统一转写接口。
///
/// - `wav_bytes`：16kHz 单声道 WAV 文件字节
/// - `context`：可选的上下文偏置（如暂存条现有文本），
///   支持该能力的 Provider 可用它缓解专业词误识别（PRD 6.4）
#[async_trait]
pub trait AsrProvider: Send + Sync {
    async fn transcribe(&self, wav_bytes: &[u8], context: Option<&str>) -> Result<String>;
}

/// 根据配置构造 ASR Provider。Key 缺失时返回 None。
pub fn provider_from_config(cfg: &Config) -> Option<Arc<dyn AsrProvider>> {
    let key = cfg
        .dashscope_api_key
        .clone()
        .filter(|k| !k.trim().is_empty())?;
    Some(Arc::new(bailian::BailianAsr::new(key, cfg.asr_model.clone())))
}
