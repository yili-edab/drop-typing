//! 用户级配置。
//!
//! 配置文件查找顺序（第一个存在的生效）：
//! 1. `~/.napkeys.toml`
//! 2. `~/.break-your-keyboard.toml`（旧代号遗留，向后兼容，读取时提示改名）
//!
//! API Key 解析顺序：`[asr].api_key` → 旧版顶层 `dashscope_api_key` → 环境变量 `DASHSCOPE_API_KEY`。

use std::path::PathBuf;

use serde::Deserialize;

fn default_asr_provider() -> String {
    "bailian".to_string()
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

/// ASR 配置。
///
/// `provider` 是厂商名（如 `bailian`），用于文档/分组；`protocol` 决定代码用哪个
/// 适配器：
/// - `dashscope-realtime`（默认）：DashScope 原生 WebSocket 流式协议（fun-asr-realtime）
/// - `dashscope-http`：DashScope HTTP 同步接口（qwen3-asr-flash，M1 方案，备选）
///
/// `protocol` 缺省时按旧版 `provider` 写法推断（向后兼容 `bailian-realtime` /
/// `bailian-http` / `bailian`）。
#[derive(Debug, Clone, Deserialize)]
pub struct AsrConfig {
    #[serde(default = "default_asr_provider")]
    pub provider: String,
    #[serde(default)]
    pub protocol: Option<String>,
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
            protocol: None,
            model: None,
            base_url: None,
            api_key: None,
        }
    }
}

/// LLM 配置（M2 清洗层预留，M1/M1.5 仅解析不使用）。
///
/// 与 ASR 同样约定：`provider` 是厂商名，`protocol` 决定适配器
/// （如 `anthropic-messages` / `openai-chat`）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub protocol: Option<String>,
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
            v.push(home.join(".napkeys.toml"));
            // 旧代号遗留路径（产品未发布前的 M1 用户），向后兼容
            v.push(home.join(".break-your-keyboard.toml"));
        }
        v
    }

    /// 是否为旧代号遗留路径
    fn is_legacy_path(path: &std::path::Path) -> bool {
        path.file_name()
            .map(|n| n == ".break-your-keyboard.toml")
            .unwrap_or(false)
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

    /// ASR 协议（适配器选择依据）。`protocol` 缺省时按旧版 `provider` 写法推断。
    pub fn asr_protocol(&self) -> String {
        if let Some(p) = &self.asr.protocol {
            return p.clone();
        }
        match self.asr.provider.as_str() {
            // 旧版写法向后兼容
            "bailian-realtime" | "fun-asr-realtime" => "dashscope-realtime".to_string(),
            "bailian-http" => "dashscope-http".to_string(),
            // 新版默认：provider 只表厂商，未显式给 protocol 时用实时协议
            _ => "dashscope-realtime".to_string(),
        }
    }

    /// ASR 模型名（按协议给默认值）
    pub fn asr_model_name(&self) -> String {
        if let Some(m) = &self.asr.model {
            return m.clone();
        }
        if let Some(m) = &self.asr_model {
            return m.clone();
        }
        match self.asr_protocol().as_str() {
            "dashscope-http" => default_http_model(),
            _ => default_realtime_model(),
        }
    }

    /// 宽松加载：永远返回一份可用配置 + 可选的告警信息。
    pub fn load_lenient() -> (Config, Option<String>) {
        let (mut cfg, warning) = match Self::path() {
            Some(path) => match std::fs::read_to_string(&path) {
                Ok(raw) => match toml::from_str::<Config>(&raw) {
                    Ok(c) => {
                        let warn = Self::is_legacy_path(&path).then(|| {
                            "检测到旧配置文件 ~/.break-your-keyboard.toml（产品已更名为 napkeys），\
                             建议执行：mv ~/.break-your-keyboard.toml ~/.napkeys.toml"
                                .to_string()
                        });
                        (c, warn)
                    }
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
                Some(
                    "未找到配置文件 ~/.napkeys.toml。请创建并填入 ASR API Key；\
                     或设置环境变量 DASHSCOPE_API_KEY。"
                        .to_string(),
                ),
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
