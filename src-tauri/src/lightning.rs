//! 闪电指令（L1）：本地声学关键词匹配。
//!
//! 只在指令通道录音期间运行：录音 PCM 并行喂给本模块的匹配器，
//! 命中高阈值动作别名即返回对应指令，由 pipeline 作废 ASR 并立即执行。

use std::collections::HashMap;
use std::path::Path;

use crate::command::{self, KeyCombo, ParsedCommand};
use crate::config::CommandConfig;
use crate::wakeword::text2token::Text2Token;

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
