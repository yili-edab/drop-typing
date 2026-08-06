//! 闪电指令（L1）：本地声学关键词匹配。
//!
//! 只在指令通道录音期间运行：录音 PCM 并行喂给本模块的匹配器，
//! 命中高阈值动作别名即返回对应指令，由 pipeline 作废 ASR 并立即执行。

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::command::{self, KeyCombo, ParsedCommand};
use crate::config::CommandConfig;
use crate::wakeword::sherpa::SherpaKws;
use crate::wakeword::text2token::Text2Token;
use crate::wakeword::WakeWord;

/// 动作别名来源标记（设置页展示用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasSource {
    Builtin,
    User,
}

/// 一条动作别名。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasEntry {
    pub phrase: String,
    pub command: ParsedCommand,
    pub source: AliasSource,
}

/// 归一化短语：与 config 唯一性校验同一规则
/// （去首尾空格、全角转半角、ASCII 小写）。
pub fn normalize_phrase(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.trim().chars() {
        let c = if ('\u{FF01}'..='\u{FF5E}').contains(&c) {
            char::from_u32(c as u32 - 0xFEE0).unwrap_or(c)
        } else {
            c
        };
        out.extend(c.to_lowercase());
    }
    out
}

fn from_config_action(a: &crate::config::CommandActionEntry) -> ParsedCommand {
    match &a.script {
        Some(s) if !s.trim().is_empty() => ParsedCommand::Script(s.clone()),
        _ => ParsedCommand::Combo(KeyCombo {
            modifiers: a.modifiers.clone(),
            key: a.key.clone(),
        }),
    }
}

/// 合并内置 + 用户动作别名：用户条目覆盖同名内置，去重。
pub fn all_aliases(cfg: &CommandConfig) -> Vec<AliasEntry> {
    let mut map: HashMap<String, AliasEntry> = HashMap::new();
    for (phrase, cmd) in command::builtin_action_aliases() {
        map.insert(
            normalize_phrase(&phrase),
            AliasEntry {
                phrase,
                command: cmd,
                source: AliasSource::Builtin,
            },
        );
    }
    for a in &cfg.action {
        let p = a.phrase.trim();
        if p.is_empty() {
            continue;
        }
        map.insert(
            normalize_phrase(p),
            AliasEntry {
                phrase: p.to_string(),
                command: from_config_action(a),
                source: AliasSource::User,
            },
        );
    }
    map.into_values().collect()
}

/// 过滤掉用户在 `lightning_disabled` 中关闭的别名。
pub fn effective_aliases(cfg: &CommandConfig) -> Vec<AliasEntry> {
    let disabled: std::collections::HashSet<String> = cfg
        .lightning_disabled
        .iter()
        .map(|p| normalize_phrase(p))
        .collect();
    all_aliases(cfg)
        .into_iter()
        .filter(|a| !disabled.contains(&normalize_phrase(&a.phrase)))
        .collect()
}

/// 在 text2token 行 `... @label` 的 `@` 前插入每词阈值 `#0.70`。
fn keyword_line(line: &str, threshold: f32) -> anyhow::Result<String> {
    match line.rfind(" @") {
        Some(i) => Ok(format!(
            "{} #{:.2} {}",
            &line[..i],
            threshold,
            &line[i + 1..]
        )),
        None => anyhow::bail!("text2token 输出缺少 @label：{line}"),
    }
}

/// 闪电引擎：sherpa KWS + 短语 → 指令映射。
pub struct LightningSpotter {
    kws: SherpaKws,
    map: HashMap<String, ParsedCommand>,
}

impl LightningSpotter {
    /// 从模型目录 + 指令配置加载；任一条转换失败会让整体加载失败
    /// （调用方降级为仅 L2）。
    pub fn load(model_dir: &Path, cfg: &CommandConfig) -> anyhow::Result<Self> {
        let t2t = Text2Token::load(model_dir)?;
        let aliases = effective_aliases(cfg);
        let threshold = cfg.effective_lightning_threshold();
        let mut lines = Vec::with_capacity(aliases.len());
        for a in &aliases {
            let base = t2t.convert(&a.phrase, &a.phrase)?;
            lines.push(keyword_line(&base, threshold)?);
        }
        let keyword_map: Vec<(String, WakeWord)> = aliases
            .iter()
            .map(|a| {
                (
                    a.phrase.clone(),
                    WakeWord {
                        text: a.phrase.clone(),
                        action: "command".to_string(),
                    },
                )
            })
            .collect();
        let buf = lines.join("\n");
        // 行内已带 # 阈值；全局阈值传 1.0 作兜底（未带 # 的关键词永不触发）
        let kws = SherpaKws::load_with_buf(model_dir, &keyword_map, &buf, 1.0, 1.0)?;
        let map = aliases
            .iter()
            .map(|a| (normalize_phrase(&a.phrase), a.command.clone()))
            .collect();
        eprintln!(
            "[drop-typing] 闪电指令：已加载 {} 个关键词（阈值 {threshold:.2}）",
            aliases.len()
        );
        Ok(Self { kws, map })
    }

    pub fn create_stream(&self) -> sherpa_onnx::OnlineStream {
        self.kws.create_stream()
    }

    pub fn reset(&self, stream: &mut sherpa_onnx::OnlineStream) {
        self.kws.reset(stream);
    }

    /// 处理一帧音频，命中返回对应指令。
    pub fn process_frame(
        &self,
        stream: &mut sherpa_onnx::OnlineStream,
        frame: &[f32],
    ) -> Option<ParsedCommand> {
        let label = self.kws.process_frame_label(stream, frame)?;
        self.map
            .get(&label)
            .cloned()
            .or_else(|| {
                self.map
                    .iter()
                    .find(|(k, _)| label.contains(k.as_str()) || k.contains(&label))
                    .map(|(_, v)| v.clone())
            })
    }
}

/// 指令录音期间把 PCM 喂给闪电引擎的匹配器（线程安全，一次录音一个实例）。
pub struct LightningMatcher {
    spotter: Arc<LightningSpotter>,
    stream: Mutex<sherpa_onnx::OnlineStream>,
    fired: AtomicBool,
}

impl LightningMatcher {
    pub fn new(spotter: Arc<LightningSpotter>) -> Self {
        Self {
            stream: Mutex::new(spotter.create_stream()),
            spotter,
            fired: AtomicBool::new(false),
        }
    }

    /// 喂入一段 s16le 单声道 16kHz PCM；命中返回指令（一次录音最多一次）。
    pub fn feed(&self, pcm: &[u8]) -> Option<ParsedCommand> {
        if self.fired.load(Ordering::SeqCst) {
            return None;
        }
        let mut frames = Vec::with_capacity(pcm.len() / 2);
        for pair in pcm.chunks_exact(2) {
            let v = i16::from_le_bytes([pair[0], pair[1]]);
            frames.push(v as f32 / i16::MAX as f32);
        }
        let hit = {
            let mut stream = self.stream.lock().unwrap();
            self.spotter.process_frame(&mut stream, &frames)
        };
        if let Some(cmd) = hit {
            self.fired.store(true, Ordering::SeqCst);
            Some(cmd)
        } else {
            None
        }
    }
}

/// 音频匹配抽象：pipeline 转发器通过它喂 PCM，测试可用假实现。
pub trait AudioMatcher: Send + Sync {
    fn feed(&self, pcm: &[u8]) -> Option<ParsedCommand>;
}

impl AudioMatcher for LightningMatcher {
    fn feed(&self, pcm: &[u8]) -> Option<ParsedCommand> {
        LightningMatcher::feed(self, pcm)
    }
}

/// 从完整配置构建闪电引擎；模型缺失 / 加载失败返回 None（调用方降级为仅 L2）。
pub fn from_config(
    cfg: &crate::config::Config,
    resource_dir: &Path,
) -> Option<Arc<LightningSpotter>> {
    let model_dir =
        crate::wakeword::sherpa::resolve_model_dir(&cfg.wakeword.model_dir, resource_dir)?;
    match LightningSpotter::load(&model_dir, &cfg.command) {
        Ok(s) => Some(Arc::new(s)),
        Err(e) => {
            eprintln!("[drop-typing] 闪电指令加载失败：{e:#}（降级为仅文字识别）");
            None
        }
    }
}

/// 设置页展示用闪电清单。
pub fn settings_view(cfg: &crate::config::Config, resource_dir: &Path) -> serde_json::Value {
    let t2t = crate::wakeword::sherpa::resolve_model_dir(&cfg.wakeword.model_dir, resource_dir)
        .and_then(|d| Text2Token::load(&d).ok());
    let disabled: std::collections::HashSet<String> = cfg
        .command
        .lightning_disabled
        .iter()
        .map(|p| normalize_phrase(p))
        .collect();
    let threshold = cfg.command.effective_lightning_threshold();
    let items: Vec<serde_json::Value> = all_aliases(&cfg.command)
        .iter()
        .map(|a| {
            let token_line = t2t
                .as_ref()
                .and_then(|t| t.convert(&a.phrase, &a.phrase).ok())
                .and_then(|l| keyword_line(&l, threshold).ok());
            serde_json::json!({
                "phrase": a.phrase,
                "display": a.command.display(),
                "builtin": matches!(a.source, AliasSource::Builtin),
                "enabled": !disabled.contains(&normalize_phrase(&a.phrase)),
                "token_line": token_line,
            })
        })
        .collect();
    serde_json::json!({ "available": t2t.is_some(), "items": items })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CommandActionEntry, CommandConfig};

    #[test]
    fn normalize_phrase_handles_trim_fullwidth_lowercase() {
        assert_eq!(normalize_phrase(" Copy "), "copy");
        assert_eq!(normalize_phrase("ＣＯＰＹ"), "copy");
        assert_eq!(normalize_phrase("复制 "), "复制");
    }

    #[test]
    fn all_aliases_user_overrides_builtin() {
        let mut cfg = CommandConfig::default();
        cfg.action.push(CommandActionEntry {
            phrase: "复制".to_string(),
            modifiers: vec![],
            key: "V".to_string(),
            script: None,
        });
        let aliases = all_aliases(&cfg);
        let copy = aliases.iter().find(|a| a.phrase == "复制").unwrap();
        assert_eq!(copy.command.display(), "V");
        assert_eq!(copy.source, AliasSource::User);
        assert!(aliases.iter().any(|a| a.source == AliasSource::Builtin));
    }

    #[test]
    fn effective_aliases_filters_disabled() {
        let mut cfg = CommandConfig::default();
        cfg.lightning_disabled = vec!["复制".to_string()];
        let aliases = effective_aliases(&cfg);
        assert!(!aliases.iter().any(|a| a.phrase == "复制"));
        assert!(aliases.iter().any(|a| a.phrase == "粘贴"));
    }

    #[test]
    fn keyword_line_inserts_threshold_before_label() {
        let line = keyword_line("x ī zh ì @复制", 0.7).unwrap();
        assert_eq!(line, "x ī zh ì #0.70 @复制");
    }

    #[test]
    fn keyword_line_rejects_missing_label() {
        assert!(keyword_line("x ī zh ì", 0.7).is_err());
    }
}
