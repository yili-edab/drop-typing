//! 用户级配置。
//!
//! 配置文件路径：`~/.drop-typing.toml`
//!
//! API Key 解析顺序：`[asr].api_key` → 旧版顶层 `dashscope_api_key` → 环境变量 `DASHSCOPE_API_KEY`。

use std::path::PathBuf;

use serde::Deserialize;

use crate::command::Modifier;

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
    150
}

fn default_double_press_window() -> u64 {
    350
}

fn default_command_countdown() -> u64 {
    2000
}

// ── 语音指令词表条目类型（M4）──────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct CommandActionEntry {
    pub phrase: String,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    pub key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommandModifierEntry {
    pub phrase: String,
    pub modifier: Modifier,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommandKeyEntry {
    pub phrase: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommandStopEntry {
    pub phrase: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CommandHomophoneEntry {
    pub phrase: String,
    pub letter: String,
}

/// 语音指令配置（M4 指令通道）。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct CommandConfig {
    /// 倒计时毫秒（可选，覆盖顶层 command_countdown_ms）
    #[serde(default)]
    pub countdown_ms: Option<u64>,
    /// 动作别名：说 phrase → 按 modifiers + key
    #[serde(default)]
    pub action: Vec<CommandActionEntry>,
    /// 修饰键别名
    #[serde(default)]
    pub modifier: Vec<CommandModifierEntry>,
    /// 按键别名
    #[serde(default)]
    pub key: Vec<CommandKeyEntry>,
    /// 停用词（填充词/连接词）
    #[serde(default)]
    pub stop: Vec<CommandStopEntry>,
    /// 字母谐音（ASR 把英文字母识别成汉字时的映射）
    #[serde(default)]
    pub homophone: Vec<CommandHomophoneEntry>,
}

// Modifier 自定义 Deserialize：接受大小写不敏感 + 历史别名。
// 文档推荐只用 Cmd / Ctrl / Opt / Shift，但代码兼容所有写法。
impl<'de> Deserialize<'de> for Modifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.to_ascii_lowercase().as_str() {
            "command" | "cmd" | "meta" | "super" | "win" => Ok(Modifier::Command),
            "shift" => Ok(Modifier::Shift),
            "control" | "ctrl" | "ctl" => Ok(Modifier::Control),
            "option" | "opt" | "alt" => Ok(Modifier::Option),
            _ => Err(serde::de::Error::custom(format!(
                "无法识别的修饰键 '{s}'。可选：Cmd, Ctrl, Opt, Shift（大小写不敏感）"
            ))),
        }
    }
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

/// LLM 配置（M2 清洗层）。
///
/// 与 ASR 同样约定：`provider` 是厂商名，`protocol` 决定适配器
/// （`openai-chat` 默认 / `anthropic-messages`）。
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
    /// 优化强度档位（`conservative` / `standard`，默认 standard）
    #[serde(default)]
    pub strength: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub asr: AsrConfig,
    #[serde(default)]
    pub llm: LlmConfig,
    /// 语音指令（M4 指令通道）
    #[serde(default)]
    pub command: CommandConfig,
    /// 旧版（M1）顶层 Key，向后兼容
    #[serde(default)]
    pub dashscope_api_key: Option<String>,
    /// 旧版（M1）顶层模型名，向后兼容
    #[serde(default)]
    pub asr_model: Option<String>,
    /// 长按判定阈值（毫秒）
    #[serde(default = "default_threshold")]
    pub long_press_threshold_ms: u64,
    /// 双击清空暂存条窗口（毫秒），默认 150ms
    #[serde(default = "default_double_press_window")]
    pub double_press_window_ms: u64,
    /// 语音指令确认倒计时（毫秒，M4）：指令解析完成后在暂存条上倒计时，到 0 自动执行
    #[serde(default = "default_command_countdown")]
    pub command_countdown_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            asr: AsrConfig::default(),
            llm: LlmConfig::default(),
            dashscope_api_key: None,
            asr_model: None,
            long_press_threshold_ms: default_threshold(),
            double_press_window_ms: default_double_press_window(),
            command_countdown_ms: default_command_countdown(),
            command: CommandConfig::default(),
        }
    }
}

impl Config {
    /// 配置文件路径（仅当文件存在时返回）
    pub fn path() -> Option<PathBuf> {
        let p = dirs::home_dir()?.join(".drop-typing.toml");
        p.exists().then_some(p)
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

    /// LLM API Key（仅配置文件；未配置即关闭清洗层，ASR 直出）
    pub fn llm_api_key(&self) -> Option<String> {
        self.llm.api_key.clone().filter(|k| !k.trim().is_empty())
    }

    /// LLM 协议（适配器选择依据）。缺省为 PRD 6.4 约定的 OpenAI 兼容协议。
    pub fn llm_protocol(&self) -> String {
        self.llm
            .protocol
            .clone()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| "openai-chat".to_string())
    }

    /// 优化强度档位（默认 standard）
    pub fn llm_strength(&self) -> String {
        self.llm
            .strength
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "standard".to_string())
    }

    /// 指令通道有效倒计时（M4）：`[command].countdown_ms` 优先，否则顶层字段兜底
    pub fn effective_command_countdown_ms(&self) -> u64 {
        self.command.countdown_ms.unwrap_or(self.command_countdown_ms)
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
                Some(
                    "未找到配置文件 ~/.drop-typing.toml。请创建并填入 ASR API Key；\
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
