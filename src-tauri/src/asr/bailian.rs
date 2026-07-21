//! 阿里百炼（DashScope）Qwen3-ASR-Flash 适配器。
//!
//! 同步接口：POST /api/v1/services/aigc/multimodal-generation/generation
//! 音频以 base64 data URL 直传，适合"按住说话"的短音频。

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};

use super::AsrProvider;

const ENDPOINT: &str =
    "https://dashscope.aliyuncs.com/api/v1/services/aigc/multimodal-generation/generation";

pub struct BailianAsr {
    client: reqwest::Client,
    api_key: String,
    model: String,
}

impl BailianAsr {
    pub fn new(api_key: String, model: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model,
        }
    }

    fn build_body(&self, wav_bytes: &[u8]) -> Value {
        let b64 = base64::engine::general_purpose::STANDARD.encode(wav_bytes);
        json!({
            "model": self.model,
            "input": {
                "messages": [
                    {
                        "role": "user",
                        "content": [
                            { "audio": format!("data:audio/wav;base64,{b64}") }
                        ]
                    }
                ]
            }
        })
    }

    /// 从响应中提取文本。兼容 content 为字符串或 [{text: ...}] 数组两种形态。
    fn extract_text(v: &Value) -> Result<String> {
        let content = &v["output"]["choices"][0]["message"]["content"];
        match content {
            Value::String(s) => Ok(s.clone()),
            Value::Array(parts) => {
                let text: String = parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect();
                Ok(text)
            }
            _ => bail!("无法解析 DashScope 响应：{v}"),
        }
    }
}

#[async_trait]
impl AsrProvider for BailianAsr {
    async fn transcribe(&self, wav_bytes: &[u8], _context: Option<&str>) -> Result<String> {
        // TODO(M2+)：qwen3-asr-flash 支持上下文偏置（热词/背景段落），
        // 可利用 _context（暂存条现有文本）缓解专业词误识别，见 PRD 6.4。
        let body = self.build_body(wav_bytes);

        let resp = self
            .client
            .post(ENDPOINT)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .context("DashScope 请求发送失败（网络问题？）")?;

        let status = resp.status();
        let v: Value = resp
            .json()
            .await
            .context("DashScope 响应不是合法 JSON")?;

        if !status.is_success() {
            let code = v["code"].as_str().unwrap_or("unknown");
            let msg = v["message"].as_str().unwrap_or("no message");
            bail!("DashScope HTTP {status}（{code}）：{msg}");
        }

        let text = Self::extract_text(&v)?;
        if text.trim().is_empty() {
            return Err(anyhow!("ASR 返回空文本（原始响应：{v}）"));
        }
        Ok(text)
    }
}
