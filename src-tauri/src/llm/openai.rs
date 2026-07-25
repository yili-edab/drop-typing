//! OpenAI Chat Completions 兼容协议适配器（PRD 6.4 的统一约定）。
//!
//! 一套实现兼容 DeepSeek / OpenAI / Qwen / Ollama（本地）。
//! 缺省 base_url `https://api.deepseek.com`、缺省 model `deepseek-chat`。

use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::{json, Value};

use super::{post_process, repair_system_prompt, system_prompt, Strength, TextCleaner};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_BASE_URL: &str = "https://api.deepseek.com";
const DEFAULT_MODEL: &str = "deepseek-chat";

pub struct OpenAiCleaner {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl OpenAiCleaner {
    pub fn new(api_key: String, model: Option<String>, base_url: Option<&str>) -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
            api_key,
            model: model
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            base_url: base_url
                .filter(|u| !u.trim().is_empty())
                .unwrap_or(DEFAULT_BASE_URL)
                .trim_end_matches('/')
                .to_string(),
        }
    }
}

#[async_trait]
impl TextCleaner for OpenAiCleaner {
    async fn clean(&self, text: &str, strength: Strength) -> Result<String> {
        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system_prompt(strength) },
                { "role": "user", "content": text }
            ]
        });

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
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

        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow!("无法解析 LLM 响应：{v}"))?
            .trim();
        if text.is_empty() {
            return Err(anyhow!("LLM 返回空文本（原始响应：{v}）"));
        }
        Ok(post_process(text, strength))
    }

    async fn repair(&self, original: &str, instruction: &str) -> Result<String> {
        let user_content = format!("原文：{original}\n修正说明：{instruction}");
        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": repair_system_prompt() },
                { "role": "user", "content": user_content }
            ]
        });

        let resp = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
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

        let text = v["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| anyhow!("无法解析 LLM 响应：{v}"))?
            .trim();
        if text.is_empty() {
            return Err(anyhow!("LLM 返回空文本（原始响应：{v}）"));
        }
        // 修正结果也走 post_process 做 pangu 兜底
        Ok(post_process(text, Strength::Standard))
    }
}
