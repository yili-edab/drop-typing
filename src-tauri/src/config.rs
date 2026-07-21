//! 用户级配置。
//!
//! 配置文件查找顺序（第一个存在的生效）：
//! 1. `~/.break-your-keyboard.toml`
//! 2. `<系统配置目录>/break-your-keyboard/config.toml`
//!    （macOS：`~/Library/Application Support/break-your-keyboard/config.toml`）
//!
//! API Key 解析顺序：`[asr].api_key` → 旧版顶层 `dashscope_api_key` → 环境变量 `DASHSCOPE_API_KEY`。

use std::path::PathBuf;

use serde::Deserialize;

fn default_asr_provider() -> String {
    "bailian-realtime".to_string()
}

fn default_realtime_model() -> String {
    "fun-asr-realtime".to_string()
}

fn default_http_model() -> String {
    "qwen3-asr-flash".to_string()
}

fn default_threshold() -> u64 {
    250
}

/// ASR 配置。provider 取值：
/// - `bailian-realtime`（默认）：fun-asr-realtime，DashScope 原生 WebSocket 流式协议
/// - `bailian`：qwen3-asr-flash，DashScope HTTP 同步接口（M1 方案，备选）
#[derive(Debug, Clone, Deserialize)]
pub struct AsrConfig {
    #[serde(default = "default_asr_provider")]
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            provider: default_asr_provider(),
            model: None,
            base_url: None,
            api_key: None,
        }
    }
}

/// LLM 配置（M2 清洗层预留，M1/M1.5 仅解析不使用）
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub asr: AsrConfig,
    #[serde(default)]
    pub llm: LlmConfig,
    /// 旧版（M1）顶层 Key，向后兼容
    #[serde(default)]
    pub dashscope_api_key: Option<String>,
    /// 旧版（M1）顶层模型名，向后兼容
    #[serde(default)]
    pub asr_model: Option<String>,
    /// 长按判定阈值（毫秒）
    #[serde(default = "default_threshold")]
    pub long_press_threshold_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            asr: AsrConfig::default(),
            llm: LlmConfig::default(),
            dashscope_api_key: None,
            asr_model: None,
            long_press_threshold_ms: default_threshold(),
        }
    }
}

impl Config {
    /// 候选配置文件路径（按优先级）
    pub fn candidate_paths() -> Vec<PathBuf> {
        let mut v = Vec::new();
        if let Some(home) = dirs::home_dir() {
            v.push(home.join(".break-your-keyboard.toml"));
        }
        if let Some(cfg) = dirs::config_dir() {
            v.push(cfg.join("break-your-keyboard").join("config.toml"));
        }
        v
    }

    /// 实际使用的配置文件路径（第一个存在的）
    pub fn path() -> Option<PathBuf> {
        Self::candidate_paths().into_iter().find(|p| p.exists())
    }

    /// ASR API Key（含 legacy / 环境变量回退）
    pub fn asr_api_key(&self) -> Option<String> {
        self.asr
            .api_key
            .clone()
            .or_else(|| self.dashscope_api_key.clone())
            .filter(|k| !k.trim().is_empty())
    }

    /// ASR 模型名（按 provider 给默认值）
    pub fn asr_model_name(&self) -> String {
        if let Some(m) = &self.asr.model {
            return m.clone();
        }
        if let Some(m) = &self.asr_model {
            return m.clone();
        }
        match self.asr.provider.as_str() {
            "bailian" | "bailian-http" => default_http_model(),
            _ => default_realtime_model(),
        }
    }

    /// 宽松加载：永远返回一份可用配置 + 可选的告警信息。
    pub fn load_lenient() -> (Config, Option<String>) {
        let (mut cfg, warning) = match Self::path() {
            Some(path) => match std::fs::read_to_string(&path) {
                Ok(raw) => match toml::from_str::<Config>(&raw) {
                    Ok(c) => (c, None),
                    Err(e) => (
                        Config::default(),
                        Some(format!(
                            "配置文件解析失败（{}）：{e}。请检查格式，或参考 config.example.toml。",
                            path.display()
                        )),
                    ),
                },
                Err(e) => (
                    Config::default(),
                    Some(format!("配置文件读取失败（{}）：{e}", path.display())),
                ),
            },
            None => (
                Config::default(),
                Some(format!(
                    "未找到配置文件（{}）。请创建并填入 ASR API Key；或设置环境变量 DASHSCOPE_API_KEY。",
                    Self::candidate_paths()
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect::<Vec<_>>()
                        .join(" 或 ")
                )),
            ),
        };

        // 环境变量兜底
        if cfg.asr_api_key().is_none() {
            if let Ok(key) = std::env::var("DASHSCOPE_API_KEY") {
                if !key.trim().is_empty() {
                    cfg.asr.api_key = Some(key);
                }
            }
        }

        (cfg, warning)
    }
}
