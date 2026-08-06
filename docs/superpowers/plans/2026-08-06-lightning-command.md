# 闪电指令（L1 本地声学快速指令）实现计划

> **面向 AI 代理的工作者：** 必需子技能：使用 superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 逐任务实现此计划。步骤使用复选框（`- [ ]`）语法来跟踪进度。

**目标：** 在指令通道录音期间并行运行本地 KWS 声学匹配，高阈值命中动作别名即立即执行（无 ASR、无倒计时），并在设置页展示 / 开关 / 校验闪电指令。

**架构：** 独立 `LightningSpotter`（sherpa-onnx KWS + text2token，每个关键词带 `#阈值`）在指令录音时与云端 ASR 并行；命中经 mpsc 通知 pipeline 作废 ASR 并立即执行。动作别名唯一性由后端权威校验 + 前端行内校验；设置页「语音控制」面板新增「闪电指令」区块，与「动作别名」表格开关双向联动。

**技术栈：** Rust / Tauri 2、sherpa-onnx 1.13、text2token、原生 TypeScript + Shoelace。

---

## 文件结构

| 文件 | 职责 |
| --- | --- |
| `src-tauri/src/command/mod.rs` | 新增 `builtin_action_aliases()`：暴露内置动作别名（唯一性校验 / 闪电词表 / 设置页共用） |
| `src-tauri/src/config.rs` | `CommandConfig` 新增 `lightning_threshold` / `lightning_disabled`；唯一性校验与残留清理 |
| `src-tauri/src/lightning.rs`（新建） | 闪电词表合并、keyword 行生成、`LightningSpotter` / `LightningMatcher` / `AudioMatcher`、设置页视图 |
| `src-tauri/src/wakeword/sherpa.rs` | 抽出 `process_frame_label()` 返回原始命中 label，供闪电引擎复用 |
| `src-tauri/src/lib.rs` | 注册 `pub mod lightning;` |
| `src-tauri/src/pipeline.rs` | 音频转发器支持闪电匹配器；指令录音双轨；命中处理；`RuntimeState` 持有闪电引擎 |
| `src-tauri/src/settings.rs` | `get-command-config` 返回闪电清单；保存时唯一性校验 + 残留清理；配置文件面板同样校验 |
| `settings.html` / `src/settings.ts` / `src/settings.css` | 「闪电指令」区块 + 动作别名行内闪电开关 + 行内唯一性校验 |
| `config.example.toml` | 新增闪电指令配置说明 |

---

## 任务 1：暴露内置动作别名（command/mod.rs）

**文件：**
- 修改：`src-tauri/src/command/mod.rs`（在 `parse()` 之前新增函数）
- 测试：`src-tauri/src/command/mod.rs` tests 模块

- [ ] **步骤 1：编写失败的测试**

在 `command/mod.rs` 的 `mod tests` 内新增：

```rust
#[test]
fn builtin_action_aliases_are_complete() {
    let aliases = super::builtin_action_aliases();
    assert_eq!(aliases.len(), 15);
    for p in [
        "复制", "拷贝", "copy", "粘贴", "黏贴", "paste", "剪切", "cut",
        "撤销", "undo", "重做", "redo", "全选", "保存", "save",
    ] {
        assert!(aliases.iter().any(|(phrase, _)| phrase == p), "缺少内置动作别名：{p}");
    }
    let (_, cmd) = aliases.iter().find(|(p, _)| p == "复制").unwrap();
    assert_eq!(cmd.display(), "CMD+C");
    let (_, cmd) = aliases.iter().find(|(p, _)| p == "重做").unwrap();
    assert_eq!(cmd.display(), "SHIFT+CMD+Z");
}
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cd src-tauri && cargo test builtin_action_aliases_are_complete`
预期：编译失败，报错 `cannot find function 'builtin_action_aliases'`

- [ ] **步骤 3：实现最少代码**

在 `parse()` 函数之前新增：

```rust
/// 内置动作别名（短语 + 解析结果），供唯一性校验 / 闪电词表 / 设置页使用。
pub fn builtin_action_aliases() -> Vec<(String, ParsedCommand)> {
    let lex = Lexicon::build(None);
    lex.main
        .iter()
        .filter_map(|(phrase, entry)| match entry {
            LexOwned::Action(m, k, s) => {
                let cmd = match s {
                    Some(script) if !script.trim().is_empty() => {
                        ParsedCommand::Script(script.clone())
                    }
                    _ => ParsedCommand::Combo(KeyCombo {
                        modifiers: m.clone(),
                        key: k.clone(),
                    }),
                };
                Some((phrase.clone(), cmd))
            }
            _ => None,
        })
        .collect()
}
```

- [ ] **步骤 4：运行测试验证通过**

运行：`cd src-tauri && cargo test builtin_action_aliases_are_complete`
预期：`test builtin_action_aliases_are_complete ... ok`

- [ ] **步骤 5：Commit**

```bash
git add src-tauri/src/command/mod.rs
git commit -m "feat(command): 暴露内置动作别名清单"
```

---

## 任务 2：配置字段 + 动作别名唯一性校验（config.rs）

**文件：**
- 修改：`src-tauri/src/config.rs`
- 测试：`src-tauri/src/config.rs` tests 模块

- [ ] **步骤 1：编写失败的测试**

在 `config.rs` 的 tests 模块新增：

```rust
#[test]
fn command_config_lightning_defaults() {
    let cfg = CommandConfig::default();
    assert_eq!(cfg.lightning_threshold, None);
    assert!(cfg.lightning_disabled.is_empty());
    assert_eq!(cfg.effective_lightning_threshold(), 0.7);
}

#[test]
fn command_config_lightning_roundtrip() {
    let raw = "[command]\nlightning_threshold = 0.6\nlightning_disabled = [\"复制\"]\n";
    let cfg = parse_config_file(raw).expect("合法 TOML 应解析成功");
    assert_eq!(cfg.command.lightning_threshold, Some(0.6));
    assert_eq!(cfg.command.effective_lightning_threshold(), 0.6);
    assert_eq!(cfg.command.lightning_disabled, vec!["复制".to_string()]);
}

#[test]
fn lightning_threshold_out_of_range_falls_back() {
    let mut cfg = CommandConfig::default();
    cfg.lightning_threshold = Some(1.5);
    assert_eq!(cfg.effective_lightning_threshold(), 0.7);
    cfg.lightning_threshold = Some(-0.1);
    assert_eq!(cfg.effective_lightning_threshold(), 0.7);
}

#[test]
fn validate_action_uniqueness_rejects_builtin_collision() {
    let mut cfg = CommandConfig::default();
    cfg.action.push(CommandActionEntry {
        phrase: "复制".to_string(),
        modifiers: vec![],
        key: "C".to_string(),
        script: None,
    });
    let err = validate_action_uniqueness(&cfg).unwrap_err();
    assert!(err.contains("复制"), "应指向内置别名：{err}");

    let mut cfg = CommandConfig::default();
    cfg.action.push(CommandActionEntry {
        phrase: "Copy".to_string(),
        modifiers: vec![],
        key: "C".to_string(),
        script: None,
    });
    let err = validate_action_uniqueness(&cfg).unwrap_err();
    assert!(err.contains("copy"), "大小写归一化后应判重：{err}");
}

#[test]
fn validate_action_uniqueness_rejects_user_duplicate() {
    let mut cfg = CommandConfig::default();
    cfg.action.push(CommandActionEntry {
        phrase: "截图".to_string(),
        modifiers: vec![],
        key: "C".to_string(),
        script: None,
    });
    cfg.action.push(CommandActionEntry {
        phrase: "截图 ".to_string(),
        modifiers: vec![],
        key: "V".to_string(),
        script: None,
    });
    let err = validate_action_uniqueness(&cfg).unwrap_err();
    assert!(err.contains("重复"));
}

#[test]
fn validate_action_uniqueness_accepts_unique_and_ignores_empty() {
    let mut cfg = CommandConfig::default();
    cfg.action.push(CommandActionEntry {
        phrase: "截图".to_string(),
        modifiers: vec![],
        key: "C".to_string(),
        script: None,
    });
    cfg.action.push(CommandActionEntry {
        phrase: "跑备份".to_string(),
        modifiers: vec![],
        key: "C".to_string(),
        script: Some("backup.sh".to_string()),
    });
    cfg.action.push(CommandActionEntry {
        phrase: "  ".to_string(),
        modifiers: vec![],
        key: "C".to_string(),
        script: None,
    });
    assert!(validate_action_uniqueness(&cfg).is_ok());
}

#[test]
fn prune_lightning_disabled_removes_stale_entries() {
    let mut cfg = CommandConfig::default();
    cfg.lightning_disabled = vec!["复制".to_string(), "已删除的别名".to_string()];
    prune_lightning_disabled(&mut cfg);
    assert_eq!(cfg.lightning_disabled, vec!["复制".to_string()]);
}
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cd src-tauri && cargo test command_config_lightning validate_action_uniqueness prune_lightning_disabled`
预期：编译失败，报错字段 / 函数不存在

- [ ] **步骤 3：实现最少代码**

在 `CommandConfig` 结构体末尾（`homophone` 字段之后）新增：

```rust
    /// 闪电指令统一触发阈值（默认 0.7；仅动作别名生效）
    #[serde(default)]
    pub lightning_threshold: Option<f32>,
    /// 用户关闭闪电的短语（内置 + 用户动作别名通用；只影响闪电，不影响文字解析）
    #[serde(default)]
    pub lightning_disabled: Vec<String>,
```

在 `CommandConfig` 结构体之后新增 impl 块：

```rust
impl CommandConfig {
    /// 闪电指令有效阈值（未配置或越界时回退默认 0.7）
    pub fn effective_lightning_threshold(&self) -> f32 {
        match self.lightning_threshold {
            Some(v) if v > 0.0 && v <= 1.0 => v,
            _ => 0.7,
        }
    }
}
```

在 `parse_config_file()` 之后新增：

```rust
/// 动作别名短语归一化：去首尾空格、全角转半角、ASCII 小写。
fn normalize_alias_phrase(s: &str) -> String {
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

/// 校验用户动作别名唯一性（仅动作别名命名空间）：
/// 1. 不能与内置动作别名重复；2. 用户动作别名之间不能重复。
pub fn validate_action_uniqueness(cfg: &CommandConfig) -> Result<(), String> {
    use std::collections::HashMap;
    let mut seen: HashMap<String, String> = HashMap::new();
    for (phrase, _) in crate::command::builtin_action_aliases() {
        seen.insert(normalize_alias_phrase(&phrase), phrase);
    }
    for a in &cfg.action {
        let p = a.phrase.trim();
        if p.is_empty() {
            continue;
        }
        let key = normalize_alias_phrase(p);
        if let Some(existing) = seen.get(&key) {
            return Err(format!(
                "动作别名「{}」与「{}」重复：指令名字需要唯一",
                a.phrase, existing
            ));
        }
        seen.insert(key, a.phrase.clone());
    }
    Ok(())
}

/// 保存时清理 lightning_disabled 中已不存在的短语（改名 / 删除后残留）。
pub fn prune_lightning_disabled(cfg: &mut CommandConfig) {
    use std::collections::HashSet;
    let mut valid: HashSet<String> = HashSet::new();
    for (phrase, _) in crate::command::builtin_action_aliases() {
        valid.insert(normalize_alias_phrase(&phrase));
    }
    for a in &cfg.action {
        if !a.phrase.trim().is_empty() {
            valid.insert(normalize_alias_phrase(&a.phrase));
        }
    }
    cfg.lightning_disabled
        .retain(|p| valid.contains(&normalize_alias_phrase(p)));
}
```

- [ ] **步骤 4：运行测试验证通过**

运行：`cd src-tauri && cargo test command_config_lightning validate_action_uniqueness prune_lightning_disabled`
预期：全部 `ok`

- [ ] **步骤 5：Commit**

```bash
git add src-tauri/src/config.rs
git commit -m "feat(config): 闪电指令配置字段与动作别名唯一性校验"
```

---

## 任务 3：闪电模块纯逻辑（lightning.rs 新建）

**文件：**
- 创建：`src-tauri/src/lightning.rs`
- 修改：`src-tauri/src/lib.rs`（注册模块）
- 测试：`src-tauri/src/lightning.rs` tests 模块

- [ ] **步骤 1：编写失败的测试**

创建 `src-tauri/src/lightning.rs`，先写测试（模块体最小化到能编译测试即可，例如先只放 `mod tests` 与占位函数骨架；本步骤先把测试完整写入）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{KeyCombo, Modifier, ParsedCommand};
    use crate::config::{CommandActionEntry, CommandConfig};

    fn combo(mods: Vec<Modifier>, key: &str) -> ParsedCommand {
        ParsedCommand::Combo(KeyCombo {
            modifiers: mods,
            key: key.to_string(),
        })
    }

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
```

同时在 `src-tauri/src/lib.rs` 的模块列表（`pub mod inject;` 之后）新增：

```rust
pub mod lightning;
```

- [ ] **步骤 2：运行测试验证失败**

运行：`cd src-tauri && cargo test normalize_phrase all_aliases effective_aliases keyword_line`
预期：编译失败，报错函数 / 类型不存在

- [ ] **步骤 3：实现最少代码**

在 `src-tauri/src/lightning.rs` 中实现（测试之上）：

```rust
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
```

并把第 1 步的 `mod tests` 放在文件末尾。

- [ ] **步骤 4：运行测试验证通过**

运行：`cd src-tauri && cargo test normalize_phrase all_aliases effective_aliases keyword_line`
预期：全部 `ok`

- [ ] **步骤 5：Commit**

```bash
git add src-tauri/src/lightning.rs src-tauri/src/lib.rs
git commit -m "feat(lightning): 闪电指令词表合并与关键词行生成"
```

---

## 任务 4：sherpa 命中 label 接口 + 闪电引擎加载

**文件：**
- 修改：`src-tauri/src/wakeword/sherpa.rs`
- 修改：`src-tauri/src/lightning.rs`

- [ ] **步骤 1：重构 `process_frame`，抽出 `process_frame_label`**

在 `SherpaKws` 中新增：

```rust
    /// 解码一帧音频并返回命中的原始 keyword（trim + 小写）；
    /// 命中后自动重置 stream。
    pub fn process_frame_label(
        &self,
        stream: &mut sherpa_onnx::OnlineStream,
        frame: &[f32],
    ) -> Option<String> {
        stream.accept_waveform(16_000, frame);
        let mut decode_iters: u32 = 0;
        while self.spotter.is_ready(stream) {
            self.spotter.decode(stream);
            decode_iters += 1;
            if decode_iters > 500 {
                eprintln!(
                    "[drop-typing] 唤醒词：decode 迭代超过上限（{}），强制跳出",
                    decode_iters,
                );
                self.spotter.reset(stream);
                return None;
            }
        }
        let result: Option<KeywordResult> = self.spotter.get_result(stream);
        match result {
            Some(r) if !r.keyword.is_empty() => {
                let detected = r.keyword.trim().to_lowercase();
                self.spotter.reset(stream);
                Some(detected)
            }
            _ => None,
        }
    }
```

然后把原 `process_frame` 的 `accept_waveform / decode / get_result / reset` 部分替换为：

```rust
    pub fn process_frame(
        &self,
        stream: &mut sherpa_onnx::OnlineStream,
        frame: &[f32],
    ) -> Option<WakeWord> {
        let detected = self.process_frame_label(stream, frame)?;
        // 以下为原匹配逻辑（exact / contains）+ 日志，保持不变
        let find_exact = |map: &HashMap<String, WakeWord>| -> Option<WakeWord> {
            map.iter()
                .find(|(k, _)| k.trim().to_lowercase() == detected)
                .map(|(_, w)| w.clone())
        };
        let find_contains = |map: &HashMap<String, WakeWord>| -> Option<WakeWord> {
            map.iter()
                .find(|(k, _)| {
                    let normalized = k.trim().to_lowercase();
                    detected.contains(&normalized) || normalized.contains(&detected)
                })
                .map(|(_, w)| w.clone())
        };
        let wake_word = find_exact(&self.keyword_map)
            .or_else(|| find_contains(&self.keyword_map));
        match &wake_word {
            Some(ww) => {
                eprintln!(
                    "[drop-typing] 🎤 唤醒词匹配成功：keyword='{}' action='{}'",
                    ww.text, ww.action,
                );
            }
            None => {
                let keys: Vec<&str> = self.keyword_map.keys().map(|s| s.as_str()).collect();
                eprintln!(
                    "[drop-typing] ⚠ 唤醒词匹配失败！r.keyword='{}'，keyword_map keys={:?}",
                    detected, keys,
                );
            }
        }
        wake_word
    }
```

- [ ] **步骤 2：运行现有测试验证无回归**

运行：`cd src-tauri && cargo test`
预期：现有测试全部通过（含 `find_in_dirs_locates_model_dir` 与 forwarder 测试）

- [ ] **步骤 3：在 lightning.rs 新增闪电引擎与匹配器**

在 `src-tauri/src/lightning.rs` 顶部 `use` 区补充：

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::wakeword::sherpa::SherpaKws;
use crate::wakeword::WakeWord;
```

在 `settings_view` 之前新增：

```rust
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
```

- [ ] **步骤 4：运行测试与编译检查**

```bash
cd src-tauri && cargo test && cargo check
```

预期：全部通过（`LightningSpotter::load` 的模型加载留给任务 10 手动清单验证）

- [ ] **步骤 5：Commit**

```bash
git add src-tauri/src/wakeword/sherpa.rs src-tauri/src/lightning.rs
git commit -m "feat(lightning): 闪电引擎与匹配器（复用 process_frame_label）"
```

---

## 任务 5：音频转发器支持闪电匹配

**文件：**
- 修改：`src-tauri/src/pipeline.rs`（`spawn_audio_forwarder` + 现有测试）
- 测试：`src-tauri/src/pipeline.rs` tests 模块

- [ ] **步骤 1：编写失败的测试**

在 `pipeline.rs` tests 模块新增假匹配器与测试：

```rust
    struct FakeMatcher {
        hits: StdMutex<Vec<Vec<u8>>>,
    }

    impl crate::lightning::AudioMatcher for FakeMatcher {
        fn feed(&self, pcm: &[u8]) -> Option<command::ParsedCommand> {
            if self.hits.lock().unwrap().iter().any(|h| h.as_slice() == pcm) {
                Some(command::ParsedCommand::Combo(command::KeyCombo {
                    modifiers: vec![command::Modifier::Command],
                    key: "C".to_string(),
                }))
            } else {
                None
            }
        }
    }

    #[test]
    fn forwarder_reports_lightning_hit() {
        let (pcm_tx, pcm_rx) = mpsc::channel::<Vec<u8>>();
        let (fwd_tx, fwd_rx) = mpsc::channel::<Arc<dyn RealtimeSession>>();
        let (hit_tx, hit_rx) = mpsc::channel::<command::ParsedCommand>();
        let matcher: Arc<dyn crate::lightning::AudioMatcher> = Arc::new(FakeMatcher {
            hits: StdMutex::new(vec![vec![7, 7]]),
        });
        let done_rx = spawn_audio_forwarder(pcm_rx, fwd_rx, Some(matcher), Some(hit_tx));

        pcm_tx.send(vec![1]).unwrap();
        pcm_tx.send(vec![7, 7]).unwrap();
        drop(pcm_tx);

        let sess = Arc::new(RecordingSession::new());
        fwd_tx.send(sess.clone()).unwrap();
        drop(fwd_tx);

        let hit = hit_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("应收到闪电命中");
        assert_eq!(hit.display(), "CMD+C");
        done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("转发器应完成");
    }
```

同时把现有两个 forwarder 测试的调用改为 `spawn_audio_forwarder(pcm_rx, fwd_rx, None, None)`。

- [ ] **步骤 2：运行测试验证失败**

运行：`cd src-tauri && cargo test forwarder_reports_lightning_hit`
预期：编译失败，`spawn_audio_forwarder` 参数数量不匹配

- [ ] **步骤 3：实现最少代码**

修改 `spawn_audio_forwarder`：

```rust
fn spawn_audio_forwarder(
    pcm_rx: mpsc::Receiver<Vec<u8>>,
    fwd_rx: mpsc::Receiver<Arc<dyn RealtimeSession>>,
    matcher: Option<Arc<dyn lightning::AudioMatcher>>,
    hit_tx: Option<mpsc::Sender<command::ParsedCommand>>,
) -> mpsc::Receiver<()> {
    let (done_tx, done_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf: Vec<Vec<u8>> = Vec::new();
        let mut sess: Option<Arc<dyn RealtimeSession>> = None;
        loop {
            if sess.is_none() {
                if let Ok(s) = fwd_rx.try_recv() {
                    for chunk in buf.drain(..) {
                        if let (Some(m), Some(tx)) = (&matcher, &hit_tx) {
                            if let Some(cmd) = m.feed(&chunk) {
                                let _ = tx.send(cmd);
                            }
                        }
                        if s.send_audio(&chunk).is_err() {
                            let _ = done_tx.send(());
                            return;
                        }
                    }
                    sess = Some(s);
                }
            }
            match pcm_rx.recv() {
                Ok(chunk) => {
                    if let (Some(m), Some(tx)) = (&matcher, &hit_tx) {
                        if let Some(cmd) = m.feed(&chunk) {
                            let _ = tx.send(cmd);
                        }
                    }
                    if let Some(ref s) = sess {
                        if s.send_audio(&chunk).is_err() {
                            let _ = done_tx.send(());
                            return;
                        }
                    } else {
                        buf.push(chunk);
                    }
                }
                Err(_) => break,
            }
        }
        if sess.is_none() {
            if let Ok(s) = fwd_rx.recv() {
                for chunk in buf.drain(..) {
                    if let (Some(m), Some(tx)) = (&matcher, &hit_tx) {
                        if let Some(cmd) = m.feed(&chunk) {
                            let _ = tx.send(cmd);
                        }
                    }
                    if s.send_audio(&chunk).is_err() {
                        break;
                    }
                }
            }
        }
        let _ = done_tx.send(());
    });
    done_rx
}
```

文件顶部 `use` 区已有 `use crate::command;`，无需新增。

同时把现有三处调用点更新为 `spawn_audio_forwarder(pcm_rx, fwd_rx, None, None)`：

- `HotkeyEvent::TriggerDown | RepairDown | CommandDown` 分支内
- `HotkeyEvent::MouseTriggerDown | MouseRepairDown` 分支内
- `start_wake_recording` 内

- [ ] **步骤 4：运行测试验证通过**

运行：`cd src-tauri && cargo test forwarder`
预期：`forwarder_reports_lightning_hit`、`forwarder_delivers_audio_when_session_arrives_after_recording_ends`、`forwarder_done_precedes_finish_with_all_audio` 全部 `ok`

- [ ] **步骤 5：Commit**

```bash
git add src-tauri/src/pipeline.rs
git commit -m "feat(pipeline): 音频转发器支持闪电匹配与命中上报"
```

---

## 任务 6：pipeline 指令通道双轨集成

**文件：**
- 修改：`src-tauri/src/pipeline.rs`

- [ ] **步骤 1：RuntimeState 持有闪电引擎**

修改 `RuntimeState` 结构体与 `from_config`：

```rust
struct RuntimeState {
    backend: Option<Arc<AsrBackend>>,
    cleaner: Option<Arc<dyn TextCleaner>>,
    lexicon: Arc<command::Lexicon>,
    lightning: Option<Arc<lightning::LightningSpotter>>,
    threshold: Duration,
    double_press: Duration,
    command_countdown: Duration,
}

impl RuntimeState {
    fn from_config(cfg: &Config, resource_dir: &std::path::Path) -> Self {
        Self {
            backend: asr::backend_from_config(cfg).map(Arc::new),
            cleaner: llm::cleaner_from_config(cfg),
            lexicon: Arc::new(command::Lexicon::build(Some(&cfg.command))),
            lightning: lightning::from_config(cfg, resource_dir),
            threshold: Duration::from_millis(cfg.long_press_threshold_ms),
            double_press: Duration::from_millis(cfg.double_press_window_ms),
            command_countdown: Duration::from_millis(cfg.effective_command_countdown_ms()),
        }
    }
}
```

在 `start()` 中：把 `let wake_resource_dir = app.path().resource_dir().unwrap_or_default();` 改为先取 `let resource_dir = app.path().resource_dir().unwrap_or_default();`，`RuntimeState::from_config(&cfg, &resource_dir)`，唤醒词管理线程传入 `resource_dir.clone()`。`runtime-reload` 闭包内也改为 `RuntimeState::from_config(&new_cfg, &resource_dir_for_reload)`，并在闭包外捕获 `let resource_dir_for_reload = resource_dir.clone();`。

更新测试中两处 `RuntimeState::from_config(&Config::default())` 为：

```rust
RuntimeState::from_config(&Config::default(), std::path::Path::new("/nonexistent-drop-typing-lightning-test"))
```

并新增断言：

```rust
assert!(st.lightning.is_none());
```

- [ ] **步骤 2：run_loop 快照增加 lightning**

修改循环顶部解构：

```rust
        let (backend, cleaner, lexicon, threshold, double_press, command_countdown, lightning) = {
            let g = runtime.lock().unwrap();
            (
                g.backend.clone(),
                g.cleaner.clone(),
                g.lexicon.clone(),
                g.threshold,
                g.double_press,
                g.command_countdown,
                g.lightning.clone(),
            )
        };
```

- [ ] **步骤 3：State::Recording 增加 lightning_rx**

在 `State::Recording` 结构体新增字段：

```rust
        /// 指令通道的闪电命中接收端（None = 非指令通道或闪电不可用）
        lightning_rx: Option<mpsc::Receiver<command::ParsedCommand>>,
```

更新两处 `State::Recording { ... }` 构造（键盘路径、鼠标路径）与两处松手解构（`TriggerUp/RepairUp/CommandUp`、`MouseTriggerUp/MouseRepairUp`）：鼠标路径构造 `lightning_rx: None`，解构处加 `lightning_rx: _,`。

- [ ] **步骤 4：键盘指令录音启动双轨**

在 `HotkeyEvent::TriggerDown | RepairDown | CommandDown` 分支中，把

```rust
                let mut pending_rx = None;
                let mut fwd_done_rx = None;
```

改为：

```rust
                let mut pending_rx = None;
                let mut fwd_done_rx = None;
                let mut lightning_rx = None;
```

把该分支内的 `fwd_done_rx = Some(spawn_audio_forwarder(pcm_rx, fwd_rx));` 替换为：

```rust
                        if mode == RecordMode::Command {
                            if let Some(spotter) = &lightning {
                                let matcher = Arc::new(lightning::LightningMatcher::new(
                                    spotter.clone(),
                                ));
                                let (hit_tx, hit_rx) =
                                    mpsc::channel::<command::ParsedCommand>();
                                lightning_rx = Some(hit_rx);
                                fwd_done_rx = Some(spawn_audio_forwarder(
                                    pcm_rx, fwd_rx, Some(matcher), Some(hit_tx),
                                ));
                            } else {
                                fwd_done_rx = Some(spawn_audio_forwarder(
                                    pcm_rx, fwd_rx, None, None,
                                ));
                            }
                        } else {
                            fwd_done_rx =
                                Some(spawn_audio_forwarder(pcm_rx, fwd_rx, None, None));
                        }
```

并在 `State::Recording` 构造中加入 `lightning_rx,`。

- [ ] **步骤 5：唤醒词指令录音同样双轨**

给 `start_wake_recording` 增加参数 `lightning: &Option<Arc<lightning::LightningSpotter>>`（放在 `backend` 之后），调用处（run_loop 的 `Listening` 分支）传入 `&lightning`。函数内把 `let fwd_done_rx = spawn_audio_forwarder(pcm_rx, fwd_rx);` 替换为与步骤 4 相同的 `mode == RecordMode::Command` 分支，并新增局部 `let mut lightning_rx = None;`，在 `State::Recording` 构造中加入 `lightning_rx,`。

- [ ] **步骤 6：轮询命中并立即执行**

在 run_loop 的 Timeout 分支，把

```rust
                if let State::Recording { started, mode, bar_shown, wake_finish_rx, .. } = &mut state {
```

改为：

```rust
                if let State::Recording {
                    started,
                    mode,
                    bar_shown,
                    wake_finish_rx,
                    lightning_rx,
                    tainted,
                    ..
                } = &mut state {
```

并在 `wake_finish_rx` 处理之前插入：

```rust
                    // 闪电命中：作废 ASR、立即执行（tainted 的录音不触发）
                    if !*tainted {
                        if let Some(rx) = lightning_rx {
                            if let Ok(cmd) = rx.try_recv() {
                                handle_lightning_hit(&staging, &injector, cmd, &command_gen);
                                if let Some(r) = &recorder {
                                    r.discard();
                                }
                                staging.set_recording(false);
                                staging.set_busy(false);
                                state = idle_state(&wake_buffer, &wake_rx);
                                continue;
                            }
                        }
                    } else if let Some(rx) = lightning_rx.take() {
                        drop(rx); // 作废后丢弃未读命中
                    }
```

在 `run_command` 附近新增：

```rust
/// 闪电指令命中：作废在途 ASR/倒计时，立即执行并展示短暂反馈。
fn handle_lightning_hit(
    staging: &Staging,
    injector: &Arc<dyn Injector>,
    parsed: command::ParsedCommand,
    gen: &Arc<AtomicU64>,
) {
    let my_gen = gen.fetch_add(1, Ordering::SeqCst) + 1;
    staging.set_status("");
    staging.partial("");
    staging.set_repair_note("");
    staging.clear_command();
    staging.clear_error();
    let display = parsed.display();
    staging.show_command(&display, 0);
    staging.committed();

    let staging = staging.clone();
    let injector = injector.clone();
    let gen = gen.clone();
    std::thread::spawn(move || {
        let result = match parsed {
            command::ParsedCommand::Combo(combo) => injector.simulate_combo(&combo),
            command::ParsedCommand::Script(script_value) => {
                staging.set_status("执行中");
                let r = script::run(&script_value);
                staging.set_status("");
                r
            }
        };
        match result {
            Ok(()) => {
                std::thread::sleep(Duration::from_millis(600));
                if gen.load(Ordering::SeqCst) == my_gen {
                    staging.clear_command();
                    staging.hide();
                }
            }
            Err(e) => {
                staging.clear_command();
                staging.error(&format!("闪电指令执行失败：{e:#}"));
            }
        }
    });
}
```

- [ ] **步骤 7：运行测试与编译检查**

运行：`cd src-tauri && cargo test && cargo check`
预期：全部通过；`runtime_state_defaults` / `runtime_state_reflects_custom_thresholds` / forwarder 测试均更新后通过

- [ ] **步骤 8：Commit**

```bash
git add src-tauri/src/pipeline.rs
git commit -m "feat(pipeline): 指令通道并行闪电识别，命中即执行"
```

---

## 任务 7：设置后端（闪电清单 + 保存校验）

**文件：**
- 修改：`src-tauri/src/settings.rs`

- [ ] **步骤 1：扩展 get-command-config**

把现有 `get-command-config` 处理替换为：

```rust
    // ── 语音控制（Command 词表 + 倒计时 + 闪电指令）：获取
    let ah = app.clone();
    app.listen("drop-typing://get-command-config", move |_| {
        eprintln!("[drop-typing] get-command-config received");
        let (cfg, _) = Config::load_lenient();
        let cmd = serde_json::to_value(&cfg.command).unwrap_or_default();
        let resource_dir = ah.path().resource_dir().unwrap_or_default();
        let builtin_actions: Vec<serde_json::Value> = crate::command::builtin_action_aliases()
            .iter()
            .map(|(phrase, parsed)| {
                serde_json::json!({ "phrase": phrase, "display": parsed.display() })
            })
            .collect();
        let _ = ah.emit(
            "drop-typing://command-config",
            serde_json::json!({
                "config": cmd,
                "effective_command_countdown_ms": cfg.effective_command_countdown_ms(),
                "lightning_threshold": cfg.command.effective_lightning_threshold(),
                "builtin_actions": builtin_actions,
                "lightning": crate::lightning::settings_view(&cfg, &resource_dir),
            }),
        );
    });
```

- [ ] **步骤 2：save-command-config 校验 + 清理**

把 `Ok(cmd) =>` 改为：

```rust
            Ok(mut cmd) => {
                if let Err(e) = crate::config::validate_action_uniqueness(&cmd) {
                    let _ = ah.emit(
                        "drop-typing://command-config-saved",
                        serde_json::json!({ "success": false, "error": e }),
                    );
                    return;
                }
                crate::config::prune_lightning_disabled(&mut cmd);
                let mut cfg = Config::load_lenient().0;
                cfg.command = cmd;
```

- [ ] **步骤 3：配置文件面板保存时校验**

在 `save-config-file` 的 `Ok(new_cfg) => {` 之后插入：

```rust
                if let Err(e) = crate::config::validate_action_uniqueness(&new_cfg.command) {
                    let _ = ah.emit(
                        "drop-typing://config-file-saved",
                        serde_json::json!({ "success": false, "error": format!("指令名字校验失败：{e}") }),
                    );
                    return;
                }
```

- [ ] **步骤 4：编译检查**

运行：`cd src-tauri && cargo check`
预期：编译通过

- [ ] **步骤 5：Commit**

```bash
git add src-tauri/src/settings.rs
git commit -m "feat(settings): 闪电指令清单与保存校验"
```

---

## 任务 8：设置前端（闪电指令区块 + 联动 + 行内校验）

**文件：**
- 修改：`settings.html`
- 修改：`src/settings.ts`
- 修改：`src/settings.css`

- [ ] **步骤 1：settings.html 新增区块**

在 `<label>指令确认倒计时</label>` 之前插入：

```html
          <label>闪电指令</label>
          <p class="help">按住指令键说出动作别名，本地声学识别命中即执行，不经过云端 ASR、无确认倒计时；未命中时仍走文字识别 + 倒计时。仅动作别名可注册。</p>
          <div class="field-row">
            <sl-input id="lightning-threshold" type="number" min="0.3" max="1.0" step="0.05"
                      size="small" placeholder="默认 0.7"></sl-input>
            <p class="help">识别阈值（0.3~1.0）：越高越严格、误触越少。</p>
          </div>
          <div id="lightning-list">
            <!-- JS 动态填充 -->
          </div>
          <sl-button id="btn-lightning-tokens" size="small" variant="neutral">查看 Token</sl-button>
```

- [ ] **步骤 2：settings.ts 状态与接口**

在「语音控制面板 DOM」区新增：

```ts
const lightningThreshold = document.getElementById('lightning-threshold') as any;
const lightningList = document.getElementById('lightning-list')!;
const btnLightningTokens = document.getElementById('btn-lightning-tokens') as any;
```

把接口扩展为：

```ts
interface CommandConfigState {
  countdown_ms: number | null;
  action: CommandEntry[];
  modifier: CommandEntry[];
  key: CommandEntry[];
  stop: CommandEntry[];
  homophone: CommandEntry[];
  lightning_threshold: number | null;
  lightning_disabled: string[];
}

interface LightningItem {
  phrase: string;
  display: string;
  builtin: boolean;
  enabled: boolean;
  token_line: string | null;
}
```

把 `commandConfig` 初始值改为：

```ts
let commandConfig: CommandConfigState = {
  countdown_ms: null,
  action: [],
  modifier: [],
  key: [],
  stop: [],
  homophone: [],
  lightning_threshold: null,
  lightning_disabled: [],
};
```

在 `commandConfig` 声明附近新增状态与工具函数：

```ts
let builtinActions: { phrase: string; display: string }[] = [];
let lightningItems: LightningItem[] = [];
const actionErrors = new Set<CommandEntry>();

function normalizePhrase(s: string): string {
  return s.trim()
    .replace(/[\uFF01-\uFF5E]/g, (ch) => String.fromCharCode(ch.charCodeAt(0) - 0xFEE0))
    .toLowerCase();
}

function isLightningDisabled(phrase: string): boolean {
  const key = normalizePhrase(phrase);
  return commandConfig.lightning_disabled.some((p) => normalizePhrase(p) === key);
}

function setLightningDisabled(phrase: string, disabled: boolean) {
  const key = normalizePhrase(phrase);
  const arr = commandConfig.lightning_disabled;
  const idx = arr.findIndex((p) => normalizePhrase(p) === key);
  if (disabled && idx < 0) arr.push(phrase);
  if (!disabled && idx >= 0) arr.splice(idx, 1);
  renderLightningList();
  renderLexRows('action');
}

function actionErrorText(row: CommandEntry): string {
  const p = (row.phrase || '').trim();
  if (!p) return '';
  const key = normalizePhrase(p);
  const conflict = builtinActions.find((b) => normalizePhrase(b.phrase) === key);
  if (conflict) return `与内置指令「${conflict.phrase}」重复`;
  const dup = commandConfig.action.find(
    (other) => other !== row && normalizePhrase(other.phrase || '') === key,
  );
  if (dup) return `与已有指令「${dup.phrase}」重复`;
  return '';
}

function validateActionRows(): boolean {
  const seen = new Map<string, string>();
  for (const b of builtinActions) seen.set(normalizePhrase(b.phrase), b.phrase);
  let ok = true;
  for (const row of commandConfig.action) {
    const p = (row.phrase || '').trim();
    if (!p) { actionErrors.delete(row); continue; }
    const key = normalizePhrase(p);
    if (seen.has(key)) { actionErrors.add(row); ok = false; }
    else { actionErrors.delete(row); seen.set(key, row.phrase || ''); }
  }
  return ok;
}

function renderLightningList() {
  lightningList.innerHTML = '';
  if (lightningItems.length === 0) {
    const empty = document.createElement('p');
    empty.className = 'help';
    empty.textContent = '暂无动作别名；在下方「动作别名」中添加后会自动出现在这里。';
    lightningList.appendChild(empty);
    return;
  }
  for (const item of lightningItems) {
    const enabled = !isLightningDisabled(item.phrase);
    const row = document.createElement('div');
    row.className = 'lightning-row';
    const phrase = document.createElement('span');
    phrase.className = 'lightning-phrase';
    phrase.textContent = item.phrase;
    const arrow = document.createElement('span');
    arrow.className = 'lightning-arrow';
    arrow.textContent = '→';
    const display = document.createElement('span');
    display.className = 'lightning-display';
    display.textContent = item.display;
    const badge = document.createElement('span');
    badge.className = 'lightning-badge ' + (item.builtin ? 'builtin' : 'user');
    badge.textContent = item.builtin ? '内置' : '自定义';
    const status = document.createElement('span');
    status.className = 'lightning-status';
    status.textContent = !item.token_line
      ? '未加载（仍走文字识别）'
      : (enabled ? '已加载' : '已关闭');
    const sw = document.createElement('sl-switch');
    sw.size = 'small';
    sw.checked = enabled;
    sw.addEventListener('sl-change', () => setLightningDisabled(item.phrase, !sw.checked));
    row.append(phrase, arrow, display, badge, status, sw);
    lightningList.appendChild(row);
  }
}
```

- [ ] **步骤 3：动作别名行内开关与错误提示**

在 `renderLexRows('action')` 的 `del` 按钮之前插入闪电开关：

```ts
      const lightningSw = document.createElement('sl-switch');
      lightningSw.className = 'lex-lightning';
      lightningSw.size = 'small';
      lightningSw.textContent = '闪电指令';
      lightningSw.checked = !isLightningDisabled(row.phrase || '');
      lightningSw.addEventListener('sl-change', () => {
        setLightningDisabled(row.phrase || '', !lightningSw.checked);
      });
      el.appendChild(lightningSw);
```

把 `phrase.addEventListener('sl-input', ...)` 后面追加：

```ts
      phrase.addEventListener('sl-blur', () => {
        row.phrase = (phrase.value || '').trim();
        validateActionRows();
        renderLexRows(kind);
      });
```

并在该行末尾（`wrap.appendChild(el)` 之前）追加错误提示：

```ts
    if (kind === 'action') {
      const err = document.createElement('span');
      err.className = 'lex-error';
      err.textContent = actionErrorText(row);
      err.style.display = err.textContent ? '' : 'none';
      el.appendChild(err);
    }
```

- [ ] **步骤 4：加载 / 保存 / 重置接线**

在 `drop-typing://command-config` 监听器内补充：

```ts
    builtinActions = e.payload.builtin_actions || [];
    lightningItems = e.payload.lightning?.items || [];
    commandConfig.lightning_threshold = e.payload.lightning_threshold ?? null;
    commandConfig.lightning_disabled = c.lightning_disabled || [];
    lightningThreshold.value = commandConfig.lightning_threshold
      ? String(commandConfig.lightning_threshold)
      : '';
    renderLightningList();
```

`buildCommandPayload()` 返回值补充：

```ts
    lightning_threshold: lightningThreshold.value ? parseFloat(lightningThreshold.value) : null,
    lightning_disabled: commandConfig.lightning_disabled,
```

`btnCommandSave` 点击处理开头插入：

```ts
  if (!validateActionRows()) {
    toast('danger', '存在重复的指令名字，请先修正后再保存');
    renderAllLex();
    return;
  }
```

`btnCommandReset` 的重置对象补充：

```ts
  commandConfig = {
    countdown_ms: null, action: [], modifier: [], key: [], stop: [], homophone: [],
    lightning_threshold: null, lightning_disabled: [],
  };
  lightningThreshold.value = '';
  renderLightningList();
```

`btnLightningTokens` 点击处理：

```ts
btnLightningTokens.addEventListener('click', () => {
  const content = document.getElementById('dlg-tokens-content') as HTMLPreElement;
  const lines = lightningItems.map((it) => {
    const state = isLightningDisabled(it.phrase) ? '（已关闭）' : '';
    const token = it.token_line || '（不可用：模型缺失或转换失败）';
    return `${it.phrase} → ${it.display} ${state}\n${token}`;
  });
  content.textContent = lines.join('\n\n') || '暂无闪电指令';
  (document.getElementById('dlg-tokens') as any).show();
});
```

- [ ] **步骤 5：settings.css 样式**

在 `src/settings.css` 末尾追加：

```css
#lightning-list { display: flex; flex-direction: column; gap: 6px; margin: 8px 0; }
.lightning-row { display: flex; align-items: center; gap: 8px; padding: 6px 8px; border: 1px solid var(--sl-color-neutral-200); border-radius: 6px; }
.lightning-phrase { font-weight: 600; }
.lightning-arrow { color: var(--sl-color-neutral-500); }
.lightning-display { font-family: var(--sl-font-mono); }
.lightning-badge { font-size: 12px; padding: 1px 6px; border-radius: 999px; }
.lightning-badge.builtin { background: var(--sl-color-neutral-200); }
.lightning-badge.user { background: var(--sl-color-primary-100); color: var(--sl-color-primary-700); }
.lightning-status { color: var(--sl-color-neutral-500); font-size: 12px; margin-left: auto; }
.lex-lightning { margin-left: 4px; }
.lex-error { color: var(--sl-color-danger-600); font-size: 12px; width: 100%; }
```

- [ ] **步骤 6：前端构建验证**

运行：`npm run build`
预期：`tsc` 类型检查 + vite build 通过

- [ ] **步骤 7：Commit**

```bash
git add settings.html src/settings.ts src/settings.css
git commit -m "feat(settings): 闪电指令区块、联动开关与行内唯一性校验"
```

---

## 任务 9：配置示例文档

**文件：**
- 修改：`config.example.toml`

- [ ] **步骤 1：新增闪电指令说明**

在 `config.example.toml` 的 `[command]` 注释区（`# 字母谐音` 示例之后）追加：

```toml
# ── 闪电指令（L1 本地声学快速指令）────────────────────────
# 所有动作别名（内置 + 用户）默认注册为闪电指令：按住指令键说出后，
# 本地声学识别命中即直接执行，不经过云端 ASR、无确认倒计时；
# 未命中时回退为 ASR 文字解析 + 确认倒计时。
# lightning_threshold = 0.7        # 统一触发阈值（0.3~1.0，默认 0.7；越高越严格）
# lightning_disabled = ["复制"]    # 关闭闪电的短语（只影响闪电，不影响文字解析）
```

- [ ] **步骤 2：Commit**

```bash
git add config.example.toml
git commit -m "docs: 闪电指令配置示例"
```

---

## 任务 10：验证清单

**文件：** 无代码变更

- [ ] **步骤 1：自动化验证**

```bash
cd src-tauri && cargo test && cargo check
cd .. && npm run build
```

预期：全部通过

- [ ] **步骤 2：手动清单（打包 .app 或 dev 模式均可）**

1. 指令通道（macOS 右 ⇧ / Windows Win+Shift）说「复制」→ 本地秒执行 CMD+C，无网络、无倒计时，暂存条闪现「已执行」后隐藏；
2. 说「Shift Command E」→ 仍走 L2：ASR + 1 秒倒计时；
3. 设置 → 语音控制 → 闪电指令：能看到内置 15 条 + 用户动作别名，行内开关与动作别名表格开关双向联动；
4. 关闭「复制」的闪电开关并保存 → 说「复制」回退 ASR 文字解析 + 倒计时；
5. 新增动作别名「复制」→ 输入框标红提示与内置重复，保存被拒；直接在配置文件面板写重复别名保存同样被拒；
6. 调整阈值（如 0.85）→ 误触减少、灵敏度降低；
7. 唤醒词「小易控」进入的指令录音同样触发闪电；
8. 录音期间按下其它键（组合键用法）→ 本次录音作废，闪电不触发；
9. Windows 与 macOS 各过一遍 1~8。

---

## 自检记录

- 规格覆盖：设计文档第 2~8 节均有对应任务（词表范围 → 任务 3；并行双轨 → 任务 5/6；配置 → 任务 2/9；唯一性校验 → 任务 2/7/8；设置界面 → 任务 7/8；错误处理 → 任务 6/7；验证 → 任务 10）。
- 占位符：无 TODO / 待定；所有代码步骤给出完整代码。
- 类型一致性：`lightning_rx` 在任务 6 各步骤使用一致的字段名；`spawn_audio_forwarder` 新参数顺序（matcher, hit_tx）在任务 5/6 保持一致；`effective_lightning_threshold` / `validate_action_uniqueness` / `prune_lightning_disabled` / `builtin_action_aliases` / `settings_view` / `process_frame_label` 的签名在后续任务中均按本计划定义使用。
