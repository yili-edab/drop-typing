//! 用户级配置：config.toml
//!
//! 路径：`<系统配置目录>/break-your-keyboard/config.toml`
//! macOS 上即 `~/Library/Application Support/break-your-keyboard/config.toml`。
//!
//! API Key 也可以由环境变量 `DASHSCOPE_API_KEY` 提供（配置文件优先）。

use std::path::PathBuf;

use serde::Deserialize;

fn default_asr_model() -> String {
    "qwen3-asr-flash".to_string()
}

fn default_threshold() -> u64 {
    250
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    /// 阿里百炼 DashScope API Key
    #[serde(default)]
    pub dashscope_api_key: Option<String>,
    /// ASR 模型名
    #[serde(default = "default_asr_model")]
    pub asr_model: String,
    /// 长按判定阈值（毫秒）
    #[serde(default = "default_threshold")]
    pub long_press_threshold_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            dashscope_api_key: None,
            asr_model: default_asr_model(),
            long_press_threshold_ms: default_threshold(),
        }
    }
}

impl Config {
    /// 配置文件路径
    pub fn path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("break-your-keyboard")
            .join("config.toml")
    }

    /// 宽松加载：永远返回一份可用配置 + 可选的告警信息。
    ///
    /// - 配置文件不存在 / 解析失败 → 返回默认配置 + 告警
    /// - 配置中无 Key 时回退到环境变量 `DASHSCOPE_API_KEY`
    pub fn load_lenient() -> (Config, Option<String>) {
        let path = Self::path();
        let (mut cfg, warning) = match std::fs::read_to_string(&path) {
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
            Err(_) => (
                Config::default(),
                Some(format!(
                    "未找到配置文件：{}\n请复制 config.example.toml 到该路径并填入 DashScope API Key；\
                     或设置环境变量 DASHSCOPE_API_KEY。",
                    path.display()
                )),
            ),
        };

        if cfg
            .dashscope_api_key
            .as_deref()
            .map(|k| k.trim().is_empty())
            .unwrap_or(true)
        {
            if let Ok(key) = std::env::var("DASHSCOPE_API_KEY") {
                if !key.trim().is_empty() {
                    cfg.dashscope_api_key = Some(key);
                }
            }
        }

        (cfg, warning)
    }
}
