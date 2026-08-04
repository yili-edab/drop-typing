//! 用户级配置。
//!
//! 配置文件路径：`~/.drop-typing.toml`
//!
//! API Key 解析顺序：`[asr].api_key` → 旧版顶层 `dashscope_api_key` → 环境变量 `DASHSCOPE_API_KEY`。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::command::Modifier;
use crate::hotkey::{Bindings, KeySpec, MouseBindings};

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

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CommandActionEntry {
    pub phrase: String,
    #[serde(default)]
    pub modifiers: Vec<Modifier>,
    pub key: String,
    /// 预留的脚本执行钩子：配置后该别名优先执行脚本（当前版本提示未支持，不做按键模拟）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CommandModifierEntry {
    pub phrase: String,
    pub modifier: Modifier,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CommandKeyEntry {
    pub phrase: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CommandStopEntry {
    pub phrase: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct CommandHomophoneEntry {
    pub phrase: String,
    pub letter: String,
}

/// 语音指令配置（M4 指令通道）。
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
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

// ── 唤醒词配置类型 ──────────────────────────────────────────────────

/// 单个唤醒词条目。
///
/// `keyword` 为自然语言关键词（如 "DT 打"），
/// `action` 为检测到后进入的通道。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct KeywordEntry {
    /// 自然语言关键词（如 "DT 打"、"DT 修"）
    pub keyword: String,
    /// 检测到后进入的通道："input" | "repair" | "command"
    pub action: String,
}

// ── 唤醒词总配置 ────────────────────────────────────────────────────

/// 唤醒词配置（sherpa-onnx KeywordSpotter）。
///
/// `enabled = true` 时启动持续监听 + 唤醒词检测。
/// 无有效模型时自动降级为仅热键模式。
///
/// sherpa-onnx 原生支持一个模型 + 多个 keywords，
/// 不再区分 multi/single 模式。
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct WakewordConfig {
    /// 是否开启唤醒词（默认 false）
    #[serde(default)]
    pub enabled: bool,
    /// 模型目录名或文件系统路径（默认 "sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20"）
    #[serde(default = "default_model_dir")]
    pub model_dir: String,
    /// 自定义唤醒词列表（决定哪些关键词触发哪个通道）
    #[serde(default)]
    pub keywords: Vec<KeywordEntry>,
    /// sherpa-onnx 检测阈值（全局，默认 0.25）
    #[serde(default = "default_keywords_threshold")]
    pub keywords_threshold: f32,
    /// 唤醒词 score boost（默认 1.0）
    #[serde(default = "default_keywords_score")]
    pub keywords_score: f32,
    /// 唤醒后多久静音判定录音结束（毫秒，默认 1500）
    #[serde(default = "default_silence_timeout")]
    pub silence_timeout_ms: u64,
    /// 唤醒词前保留的音频时长（毫秒，默认 500）
    #[serde(default = "default_pre_roll")]
    pub pre_roll_ms: u64,
    /// 环形缓冲区时长（毫秒，默认 3000）
    #[serde(default = "default_ring_buffer_duration")]
    pub ring_buffer_duration_ms: u64,
}

fn default_model_dir() -> String {
    "sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20".to_string()
}

fn default_keywords_threshold() -> f32 {
    0.25
}

fn default_keywords_score() -> f32 {
    1.0
}

fn default_silence_timeout() -> u64 {
    1500
}

fn default_pre_roll() -> u64 {
    500
}

fn default_ring_buffer_duration() -> u64 {
    3000
}

impl Default for WakewordConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            model_dir: default_model_dir(),
            keywords: Vec::new(),
            keywords_threshold: default_keywords_threshold(),
            keywords_score: default_keywords_score(),
            silence_timeout_ms: default_silence_timeout(),
            pre_roll_ms: default_pre_roll(),
            ring_buffer_duration_ms: default_ring_buffer_duration(),
        }
    }
}
// 文档推荐只用 Cmd / Ctrl / Opt / Shift，但代码兼容所有写法。
impl<'de> Deserialize<'de> for Modifier {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.to_ascii_lowercase().as_str() {
            "command" | "cmd" | "meta" | "super" | "win" => Ok(Modifier::Command),
            "commandleft" | "cmdleft" | "metaleft" | "superleft" | "winleft" => {
                Ok(Modifier::MetaLeft)
            }
            "commandright" | "cmdright" | "metaright" | "superright" | "winright" => {
                Ok(Modifier::MetaRight)
            }
            "shift" => Ok(Modifier::Shift),
            "shiftleft" => Ok(Modifier::ShiftLeft),
            "shiftright" => Ok(Modifier::ShiftRight),
            "control" | "ctrl" | "ctl" => Ok(Modifier::Control),
            "controlleft" | "ctrlleft" | "ctlleft" => Ok(Modifier::ControlLeft),
            "controlright" | "ctrlright" | "ctlright" => Ok(Modifier::ControlRight),
            "option" | "opt" | "alt" => Ok(Modifier::Option),
            "optionleft" | "optleft" | "altleft" => Ok(Modifier::Alt),
            "optionright" | "optright" | "altright" | "altgr" => Ok(Modifier::AltGr),
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
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
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
    /// 当前选中的润色样式（`high_eq` / `low_eq` / `anti_pua` / `pua`），
    /// None 表示不选任何样式，仅用基础润色提示词
    #[serde(default)]
    pub current_style: Option<String>,
}

/// 快捷键原始配置（TOML 中的字符串列表）。
///
/// 每条通道的按键由一组 KeySpec 字符串定义，运行时解析为 `Bindings`。
/// 未配置时使用平台默认值。
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct HotkeyRawConfig {
    /// 输入/提交通道（长按录音，短按确认）
    #[serde(default)]
    pub trigger: Option<Vec<String>>,
    /// 修正/控制通道
    #[serde(default)]
    pub repair: Option<Vec<String>>,
    /// 指令/修复通道
    #[serde(default)]
    pub command: Option<Vec<String>>,
    /// 清空暂存条
    #[serde(default)]
    pub cancel: Option<Vec<String>>,
}

impl HotkeyRawConfig {
    /// 将原始字符串列表解析为键盘 `Bindings` 部分。
    ///
    /// 未配置的通道回退为平台默认值；解析失败的按键名会以 Err 报告。
    pub fn into_keyboard_bindings(self) -> Result<Bindings, String> {
        let defaults = Bindings::platform_default();
        Ok(Bindings {
            trigger: parse_or_default(self.trigger, &defaults.trigger)?,
            repair: parse_or_default(self.repair, &defaults.repair)?,
            command: parse_or_default(self.command, &defaults.command)?,
            cancel: parse_or_default(self.cancel, &defaults.cancel)?,
            mouse: MouseBindings::default(),
        })
    }
}

fn parse_or_default(
    raw: Option<Vec<String>>,
    defaults: &[KeySpec],
) -> Result<Vec<KeySpec>, String> {
    match raw {
        Some(keys) if !keys.is_empty() => {
            keys.iter().map(|k| KeySpec::parse(k)).collect()
        }
        _ => Ok(defaults.to_vec()),
    }
}

// ── 鼠标侧键配置类型 ──────────────────────────────────────────────

/// 鼠标按键（用户配置中的 `"forward"` / `"back"`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    /// 前进键（X2 / Button 5）
    Forward,
    /// 后退键（X1 / Button 4）
    Back,
}

/// 鼠标侧键绑定配置（`[hotkey.mouse]` 段）。
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct MouseHotkeyConfig {
    /// 输入/提交通道（前进键，长按录音、短按提交）
    #[serde(default)]
    pub trigger: Option<MouseButton>,
    /// 修正通道（后退键，长按说修正指令）
    #[serde(default)]
    pub repair: Option<MouseButton>,
}

impl MouseHotkeyConfig {
    /// 转换为 `MouseBindings`。
    pub fn into_mouse_bindings(self) -> Result<MouseBindings, String> {
        use crate::hotkey::MouseButton as HkMouseButton;
        Ok(MouseBindings {
            trigger: self.trigger.map(|b| match b {
                MouseButton::Forward => HkMouseButton::Forward,
                MouseButton::Back => HkMouseButton::Back,
            }),
            repair: self.repair.map(|b| match b {
                MouseButton::Forward => HkMouseButton::Forward,
                MouseButton::Back => HkMouseButton::Back,
            }),
        })
    }
}

// ── 热键总配置（[hotkey]）────────────────────────────────────────

/// 热键总配置。
///
/// Keyboard 字段经 `#[serde(flatten)]` 散布在 `[hotkey]` 顶层
/// （与旧版配置格式完全兼容）；`[hotkey.mouse]` 是可选子表。
///
/// 示例：
/// ```toml
/// [hotkey]
/// trigger = ["MetaRight"]
/// repair  = ["AltGr"]
/// command = ["ShiftRight"]
/// cancel  = ["Escape"]
///
/// [hotkey.mouse]
/// trigger = "forward"
/// repair  = "back"
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct HotkeyConfig {
    /// 键盘快捷键（flatten 到 `[hotkey]` 顶层）
    #[serde(flatten)]
    pub keyboard: HotkeyRawConfig,
    /// 鼠标侧键（`[hotkey.mouse]`，缺省不绑）
    #[serde(default)]
    pub mouse: Option<MouseHotkeyConfig>,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            keyboard: HotkeyRawConfig::default(),
            mouse: None,
        }
    }
}

impl HotkeyConfig {
    /// 将热键配置解析为运行时 `Bindings`。
    pub fn into_bindings(self) -> Result<Bindings, String> {
        let keyboard = self.keyboard.into_keyboard_bindings()?;
        let mouse = match self.mouse {
            Some(m) => m.into_mouse_bindings()?,
            None => MouseBindings::default(),
        };
        Ok(Bindings {
            trigger: keyboard.trigger,
            repair: keyboard.repair,
            command: keyboard.command,
            cancel: keyboard.cancel,
            mouse,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct Config {
    #[serde(default)]
    pub asr: AsrConfig,
    #[serde(default)]
    pub llm: LlmConfig,
    /// 语音指令（M4 指令通道）
    #[serde(default)]
    pub command: CommandConfig,
    /// 快捷键配置（缺省使用平台默认值）
    #[serde(default)]
    pub hotkey: HotkeyConfig,
    /// 唤醒词（缺省关闭）
    #[serde(default)]
    pub wakeword: WakewordConfig,
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
            hotkey: HotkeyConfig::default(),
            wakeword: WakewordConfig::default(),
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

    /// 解析快捷键绑定。
    ///
    /// 用户配置了 `[hotkey]` 段且解析成功时使用用户配置；
    /// 解析失败时打印警告并回退为平台默认值。
    pub fn hotkey_bindings(&self) -> Bindings {
        let defaults = Bindings::platform_default();
        let has_keyboard = self.hotkey.keyboard.trigger.is_some()
            || self.hotkey.keyboard.repair.is_some()
            || self.hotkey.keyboard.command.is_some()
            || self.hotkey.keyboard.cancel.is_some();
        let has_mouse = self.hotkey.mouse.is_some();
        if !has_keyboard && !has_mouse {
            return defaults;
        }
        match self.hotkey.clone().into_bindings() {
            Ok(b) => b,
            Err(e) => {
                eprintln!(
                    "[drop-typing] 快捷键配置解析失败：{e}。已回退为平台默认值。"
                );
                defaults
            }
        }
    }

    /// 将当前配置写回 ~/.drop-typing.toml。
    /// 采用全量序列化覆盖策略（后续可改为部分合并以保留注释）。
    pub fn save(&self) -> Result<(), String> {
        let path = dirs::home_dir()
            .ok_or_else(|| "无法确定家目录".to_string())?
            .join(".drop-typing.toml");
        let text = toml::to_string_pretty(self)
            .map_err(|e| format!("配置序列化失败：{e}"))?;
        std::fs::write(&path, text).map_err(|e| format!("配置文件写入失败（{}）：{e}", path.display()))
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

/// 解析配置文件原文（TOML 语法 + 类型校验）。
///
/// 供设置页「配置文件」面板兜底编辑使用；失败返回带行列号的错误，不写盘。
pub fn parse_config_file(raw: &str) -> Result<Config, String> {
    toml::from_str::<Config>(raw).map_err(|e| format_toml_error(raw, &e))
}

/// 将 toml 错误转换为带行列号的文案。
pub fn format_toml_error(raw: &str, e: &toml::de::Error) -> String {
    if let Some(span) = e.span() {
        let pos = span.start.min(raw.len());
        let line = raw[..pos].matches('\n').count() + 1;
        let line_start = raw[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let col = pos.saturating_sub(line_start) + 1;
        format!("第 {line} 行第 {col} 列：{e}")
    } else {
        e.to_string()
    }
}

/// 判断新旧配置是否需要重启应用（热键或唤醒词段发生变化时返回 true）。
pub fn needs_restart(old: &Config, new: &Config) -> bool {
    old.hotkey != new.hotkey || old.wakeword != new.wakeword
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_config_file_rejects_invalid_toml_with_position() {
        let raw = "long_press_threshold_ms = \"abc\"\nasr = [\n";
        let err = parse_config_file(raw).unwrap_err();
        assert!(err.contains("行"), "错误应包含行列信息：{err}");
    }

    #[test]
    fn parse_config_file_accepts_valid_toml() {
        let raw = "[asr]\nprovider = \"bailian\"\napi_key = \"sk-test\"\n\n[llm]\napi_key = \"sk-llm\"\n";
        let cfg = parse_config_file(raw).expect("合法 TOML 应解析成功");
        assert_eq!(cfg.asr.provider, "bailian");
        assert_eq!(cfg.asr_api_key().as_deref(), Some("sk-test"));
    }

    #[test]
    fn parse_config_file_accepts_action_with_script() {
        let raw = "[[command.action]]\nphrase = \"跑备份\"\nmodifiers = []\nkey = \"C\"\nscript = \"/bin/sh backup.sh\"\n";
        let cfg = parse_config_file(raw).expect("合法 TOML 应解析成功");
        let a = &cfg.command.action[0];
        assert_eq!(a.phrase, "跑备份");
        assert_eq!(a.script.as_deref(), Some("/bin/sh backup.sh"));
    }

    #[test]
    fn parse_config_file_accepts_precise_modifiers() {
        let raw = "[[command.action]]\nphrase = \"测试\"\nmodifiers = [\"ControlRight\", \"ShiftLeft\"]\nkey = \"4\"\n";
        let cfg = parse_config_file(raw).expect("合法 TOML 应解析成功");
        assert_eq!(
            cfg.command.action[0].modifiers,
            vec![Modifier::ControlRight, Modifier::ShiftLeft]
        );
    }

    #[test]
    fn parse_precise_modifier_aliases() {
        let raw = "[[command.action]]\nphrase = \"测试\"\nmodifiers = [\"CmdRight\", \"AltGr\"]\nkey = \"C\"\n";
        let cfg = parse_config_file(raw).expect("合法 TOML 应解析成功");
        assert_eq!(
            cfg.command.action[0].modifiers,
            vec![Modifier::MetaRight, Modifier::AltGr]
        );
    }

    #[test]
    fn needs_restart_true_when_wakeword_changes() {
        let old = Config::default();
        let mut new = Config::default();
        new.wakeword.enabled = true;
        assert!(needs_restart(&old, &new));
    }

    #[test]
    fn needs_restart_true_when_hotkey_changes() {
        let old = Config::default();
        let mut new = Config::default();
        new.hotkey.keyboard.trigger = Some(vec!["MetaLeft".to_string()]);
        assert!(needs_restart(&old, &new));
    }

    #[test]
    fn needs_restart_false_when_only_llm_changes() {
        let old = Config::default();
        let mut new = Config::default();
        new.llm.api_key = Some("sk-x".to_string());
        assert!(!needs_restart(&old, &new));
    }
}
