//! Anthropic Messages API 兼容协议适配器。
//!
//! 适用于百炼 `/apps/anthropic` 等 Anthropic 兼容端点：
//! POST `{base_url}/v1/messages`，header `x-api-key` + `anthropic-version`。

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{post_process, repair_system_prompt, TextCleaner};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

pub struct AnthropicCleaner {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl AnthropicCleaner {
    pub fn new(api_key: String, model: Option<String>, base_url: Option<&str>) -> Result<Self> {
        let model = model
            .filter(|m| !m.trim().is_empty())
            .ok_or_else(|| anyhow!("anthropic-messages 协议需要在 [llm] 中显式配置 model"))?;
        let base_url = base_url
            .filter(|u| !u.trim().is_empty())
            .ok_or_else(|| anyhow!("anthropic-messages 协议需要在 [llm] 中显式配置 base_url"))?;
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .context("构造 HTTP client 失败")?,
            api_key,
            model,
            base_url: base_url.trim_end_matches('/').to_string(),
        })
    }
}

#[async_trait]
impl TextCleaner for AnthropicCleaner {
    async fn clean(&self, text: &str, system_prompt: &str) -> Result<String> {
        let body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": system_prompt,
            "messages": [
                { "role": "user", "content": text }
            ]
        });

        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .context("LLM 请求发送失败（网络问题？）")?;

        let status = resp.status();
        let v: Value = resp.json().await.context("LLM 响应不是合法 JSON")?;

        if !status.is_success() {
            let msg = v["error"]["message"].as_str().unwrap_or("no message");
            bail!("LLM HTTP {status}：{msg}");
        }

        // content 为 [{type:"text", text:"..."}] 数组，拼接所有 text 块
        let text: String = v["content"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect()
            })
            .ok_or_else(|| anyhow!("无法解析 LLM 响应：{v}"))?;
        let text = text.trim();
        if text.is_empty() {
            return Err(anyhow!("LLM 返回空文本（原始响应：{v}）"));
        }
        Ok(post_process(text))
    }

    async fn repair(&self, original: &str, instruction: &str) -> Result<String> {
        let user_content = format!("原文：{original}\n修正说明：{instruction}");
        let body = json!({
            "model": self.model,
            "max_tokens": 4096,
            "system": repair_system_prompt(),
            "messages": [
                { "role": "user", "content": user_content }
            ]
        });

        let resp = self
            .client
            .post(format!("{}/v1/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .context("LLM 请求发送失败（网络问题？）")?;

        let status = resp.status();
        let v: Value = resp.json().await.context("LLM 响应不是合法 JSON")?;

        if !status.is_success() {
            let msg = v["error"]["message"].as_str().unwrap_or("no message");
            bail!("LLM HTTP {status}：{msg}");
        }

        // content 为 [{type:"text", text:"..."}] 数组，拼接所有 text 块
        let text: String = v["content"]
            .as_array()
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(|t| t.as_str()))
                    .collect()
            })
            .ok_or_else(|| anyhow!("无法解析 LLM 响应：{v}"))?;
        let text = text.trim();
        if text.is_empty() {
            return Err(anyhow!("LLM 返回空文本（原始响应：{v}）"));
        }
        Ok(post_process(text))
    }
}
