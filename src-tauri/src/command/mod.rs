//! 语音指令解析（M4 指令通道）：纯本地匹配/解析，不经过 LLM（PRD 4.3）。
//!
//! 解析策略：**词表驱动的最长匹配扫描提取**（非整句精确匹配）。
//! 规范化后的文本从左到右扫描，每个位置在词表中找最长前缀匹配，提取出
//! 动作别名 / 修饰词 / 键名 / 停用词四类 token，再由 token 序列合成按键组合；
//! 匹配不上的字符记为 unknown 跳过（占比过半则整体判废，防误触发）。
//!
//! 两轮解析：第一轮不含谐音表；失败后再带谐音表跑第二轮（"命令 加 西" → CMD+C）。
//!
//! 已知取舍：
//! - 谐音映射天然有歧义，靠"最长匹配 + 两轮解析 + unknown 占比护栏"压低误判；
//!   真误判了用户也有倒计时窗口可按任意右修饰键取消（见 pipeline.rs run_command）
//! - 中文数字"一"优先映射为数字 1 而非字母 E（数字义项在主词表，谐音表在其后）
//! - "发送"→ENTER 在个别 IM（发送键是 Cmd+Enter 的）不适用，属可接受近似
//! - 词表独立在 `lexicon.rs`，便于后续做成用户可自定义配置

mod lexicon;
pub use lexicon::Lexicon;
use lexicon::LexOwned;

/// 修饰键（macOS 语义）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    Command,
    Shift,
    Control,
    Option,
}

/// 一个解析完成的按键组合：若干修饰键 + 一个目标键。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyCombo {
    pub modifiers: Vec<Modifier>,
    /// 规范化键名：单个大写字母/数字，或 ENTER / SPACE / TAB / ESC / DELETE /
    /// UP / DOWN / LEFT / RIGHT / F1..F12
    pub key: String,
}

impl KeyCombo {
    /// 展示用文本，如 "CMD+C" / "SHIFT+CMD+E" / "ENTER"
    pub fn display(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        // 按 macOS 惯例顺序 ⌃ ⌥ ⇧ ⌘ 输出
        for m in [
            Modifier::Control,
            Modifier::Option,
            Modifier::Shift,
            Modifier::Command,
        ] {
            if self.modifiers.contains(&m) {
                parts.push(match m {
                    Modifier::Control => "CTRL",
                    Modifier::Option => "OPT",
                    Modifier::Shift => "SHIFT",
                    Modifier::Command => "CMD",
                });
            }
        }
        if parts.is_empty() {
            self.key.clone()
        } else {
            format!("{}+{}", parts.join("+"), self.key)
        }
    }
}

/// 提取出的 token
enum Tok {
    Action(Vec<Modifier>, String),
    Mod(Modifier),
    Key(String),
}

/// 规范化：全角→半角、小写化；只保留 ASCII 字母数字与汉字，
/// 其余（空白 / 标点 / + - _ 等符号）统一折叠为单个空格分隔。
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for raw in text.chars() {
        let c = match raw {
            '\u{FF01}'..='\u{FF5E}' => char::from_u32(raw as u32 - 0xFEE0).unwrap_or(raw),
            '\u{3000}' => ' ',
            other => other,
        };
        for lc in c.to_lowercase() {
            if lc.is_ascii_alphanumeric() || ('\u{4E00}'..='\u{9FFF}').contains(&lc) {
                out.push(lc);
            } else {
                out.push(' ');
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 英文/数字连续段整体查主词表
fn lookup_run<'a>(run: &str, lexicon: &'a Lexicon) -> Option<&'a LexOwned> {
    lexicon
        .main
        .iter()
        .find(|(pat, _)| pat.as_str() == run)
        .map(|(_, entry)| entry)
}

/// 当前位置最长前缀匹配。主词表优先；主词表无匹配且启用谐音时才查谐音表。
fn lookup_prefix<'a>(
    rest: &str,
    use_homophones: bool,
    lexicon: &'a Lexicon,
) -> Option<&'a LexOwned> {
    let mut best: Option<&LexOwned> = None;
    let mut best_len = 0;
    for (pat, entry) in &lexicon.main {
        if pat.len() > best_len && rest.starts_with(pat.as_str()) {
            best = Some(entry);
            best_len = pat.len();
        }
    }
    if best.is_none() && use_homophones {
        for (pat, entry) in &lexicon.homophones {
            if rest.starts_with(pat.as_str()) {
                return Some(entry);
            }
        }
    }
    best
}

/// 从左到右扫描，提取 token 序列；返回 (tokens, unknown 字符数, 非空字符总数)
fn scan(text: &str, use_homophones: bool, lexicon: &Lexicon) -> (Vec<Tok>, usize, usize) {
    let total = text.chars().filter(|c| !c.is_whitespace()).count();
    let mut toks = Vec::new();
    let mut unknown = 0usize;
    let mut i = 0;
    while i < text.len() {
        let c = text[i..].chars().next().unwrap();
        if c.is_whitespace() {
            i += c.len_utf8();
            continue;
        }
        if c.is_ascii_alphanumeric() {
            // 英文/数字连续段整体查表（shift / f12 / cmd …）
            let start = i;
            while i < text.len() && text.as_bytes()[i].is_ascii_alphanumeric() {
                i += 1;
            }
            let run = &text[start..i];
            if let Some(lex) = lookup_run(run, lexicon) {
                push_tok(&mut toks, lex);
            } else if run.len() == 1 {
                // 单个字母/数字：直接作为键名
                toks.push(Tok::Key(run.to_ascii_uppercase()));
            } else {
                unknown += run.len();
            }
        } else {
            let rest = &text[i..];
            match lookup_prefix(rest, use_homophones, lexicon) {
                Some(lex) => {
                    push_tok(&mut toks, lex);
                    // 用同一逻辑重算匹配长度，保证前进的正是匹配项
                    i += matched_len(rest, use_homophones, lexicon);
                }
                None => {
                    unknown += 1;
                    i += c.len_utf8();
                }
            }
        }
    }
    (toks, unknown, total)
}

/// 与 lookup_prefix 同逻辑，返回匹配的字节长度（保证 scan 前进的正是匹配项）
fn matched_len(rest: &str, use_homophones: bool, lexicon: &Lexicon) -> usize {
    let mut best_len = 0;
    for (pat, _) in &lexicon.main {
        if pat.len() > best_len && rest.starts_with(pat.as_str()) {
            best_len = pat.len();
        }
    }
    if best_len == 0 && use_homophones {
        for (pat, _) in &lexicon.homophones {
            if rest.starts_with(pat.as_str()) {
                return pat.len();
            }
        }
    }
    best_len
}

fn push_tok(toks: &mut Vec<Tok>, lex: &LexOwned) {
    match lex {
        LexOwned::Action(m, k) => toks.push(Tok::Action(m.clone(), k.clone())),
        LexOwned::Mod(m) => toks.push(Tok::Mod(*m)),
        LexOwned::Key(k) => toks.push(Tok::Key(k.clone())),
        LexOwned::Stop => {}
    }
}

/// token 序列 → 按键组合
fn assemble(toks: Vec<Tok>, unknown: usize, total: usize) -> Option<KeyCombo> {
    // 护栏：unknown 占比过半 → 整体判废（"哒哒哒哒" 不会碰巧命中）
    if total > 0 && unknown * 2 > total {
        return None;
    }
    // 动作别名优先（"复制一下" → CMD+C）
    for t in &toks {
        if let Tok::Action(m, k) = t {
            return Some(KeyCombo {
                modifiers: m.clone(),
                key: k.clone(),
            });
        }
    }
    // 收集修饰词与键名（各自去重）
    let mut mods: Vec<Modifier> = Vec::new();
    let mut keys: Vec<String> = Vec::new();
    for t in toks {
        match t {
            Tok::Mod(m) => {
                if !mods.contains(&m) {
                    mods.push(m);
                }
            }
            Tok::Key(k) => {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
            _ => {}
        }
    }
    match keys.len() {
        // 多个不同键名 → 无法判定，放弃（防误执行）
        0 => None,
        1 => {
            let key = keys.pop().unwrap();
            // 裸单字母/数字必须带修饰词（防误触发）；长键名允许裸用
            if mods.is_empty() && key.chars().count() == 1 {
                return None;
            }
            Some(KeyCombo {
                modifiers: mods,
                key,
            })
        }
        _ => None,
    }
}

fn try_parse(normalized: &str, use_homophones: bool, lexicon: &Lexicon) -> Option<KeyCombo> {
    let (toks, unknown, total) = scan(normalized, use_homophones, lexicon);
    assemble(toks, unknown, total)
}

/// 解析语音转写文本为按键组合；未命中返回 None。
///
/// 两轮：第一轮不含谐音表；失败后再带谐音表重试。
pub fn parse(text: &str, lexicon: &Lexicon) -> Option<KeyCombo> {
    let normalized = normalize(text);
    if normalized.is_empty() {
        return None;
    }
    // 整串特例：Windows 习惯的 "Ctrl+C / Ctrl+V" 在 macOS 上映射为 ⌘C / ⌘V（PRD 4.3）
    let squashed: String = normalized.chars().filter(|c| !c.is_whitespace()).collect();
    match squashed.as_str() {
        "ctrlc" | "controlc" => {
            return Some(KeyCombo {
                modifiers: vec![Modifier::Command],
                key: "C".to_string(),
            })
        }
        "ctrlv" | "controlv" => {
            return Some(KeyCombo {
                modifiers: vec![Modifier::Command],
                key: "V".to_string(),
            })
        }
        _ => {}
    }
    try_parse(&normalized, false, lexicon).or_else(|| try_parse(&normalized, true, lexicon))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builtin_lexicon() -> Lexicon {
        Lexicon::default()
    }

    fn disp(text: &str) -> Option<String> {
        parse(text, &builtin_lexicon()).map(|c| c.display())
    }

    #[test]
    fn aliases_chinese() {
        assert_eq!(disp("复制").as_deref(), Some("CMD+C"));
        assert_eq!(disp("粘贴").as_deref(), Some("CMD+V"));
        assert_eq!(disp("回车").as_deref(), Some("ENTER"));
        assert_eq!(disp("换行").as_deref(), Some("ENTER"));
    }

    #[test]
    fn aliases_english_and_ctrl() {
        assert_eq!(disp("copy").as_deref(), Some("CMD+C"));
        assert_eq!(disp("Ctrl+C").as_deref(), Some("CMD+C"));
        assert_eq!(disp("ctrl c").as_deref(), Some("CMD+C"));
        assert_eq!(disp("Ctrl+V").as_deref(), Some("CMD+V"));
        assert_eq!(disp("enter").as_deref(), Some("ENTER"));
    }

    #[test]
    fn combo_direct() {
        assert_eq!(disp("Shift+Command+E").as_deref(), Some("SHIFT+CMD+E"));
        assert_eq!(disp("shift command e").as_deref(), Some("SHIFT+CMD+E"));
        assert_eq!(disp("Command Shift P").as_deref(), Some("SHIFT+CMD+P"));
        assert_eq!(disp("control option f5").as_deref(), Some("CTRL+OPT+F5"));
        assert_eq!(disp("cmd space").as_deref(), Some("CMD+SPACE"));
    }

    #[test]
    fn bare_named_keys() {
        assert_eq!(disp("escape").as_deref(), Some("ESC"));
        assert_eq!(disp("tab").as_deref(), Some("TAB"));
        assert_eq!(disp("up").as_deref(), Some("UP"));
        assert_eq!(disp("f12").as_deref(), Some("F12"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(super::parse("", &builtin_lexicon()).is_none());
        assert!(super::parse("今天天气不错", &builtin_lexicon()).is_none());
        assert!(super::parse("e", &builtin_lexicon()).is_none()); // 裸字母不允许
        assert!(super::parse("shift", &builtin_lexicon()).is_none()); // 只有修饰词
        assert!(super::parse("command e f", &builtin_lexicon()).is_none()); // 两个键名
        assert!(super::parse("command 今天", &builtin_lexicon()).is_none()); // 无法识别的词
    }

    // ---- 新增：中文说法 ----

    #[test]
    fn chinese_aliases_and_modifiers() {
        assert_eq!(disp("拷贝").as_deref(), Some("CMD+C"));
        assert_eq!(disp("黏贴").as_deref(), Some("CMD+V"));
        assert_eq!(disp("剪切").as_deref(), Some("CMD+X"));
        assert_eq!(disp("撤销").as_deref(), Some("CMD+Z"));
        assert_eq!(disp("重做").as_deref(), Some("SHIFT+CMD+Z"));
        assert_eq!(disp("全选").as_deref(), Some("CMD+A"));
        assert_eq!(disp("保存").as_deref(), Some("CMD+S"));
        assert_eq!(disp("控制 选项 f5").as_deref(), Some("CTRL+OPT+F5"));
        assert_eq!(disp("命令 空格").as_deref(), Some("CMD+SPACE"));
        assert_eq!(disp("命令 回车").as_deref(), Some("CMD+ENTER"));
        assert_eq!(disp("逃逸").as_deref(), Some("ESC"));
        assert_eq!(disp("方向左").as_deref(), Some("LEFT"));
        assert_eq!(disp("换挡 命令 批").as_deref(), Some("SHIFT+CMD+P")); // 谐音
    }

    #[test]
    fn filler_words_tolerated() {
        assert_eq!(
            disp("按一下 shift command e").as_deref(),
            Some("SHIFT+CMD+E")
        );
        assert_eq!(disp("帮我按 command 加 c").as_deref(), Some("CMD+C"));
        assert_eq!(disp("复制一下").as_deref(), Some("CMD+C"));
        assert_eq!(disp("按一下复制").as_deref(), Some("CMD+C"));
    }

    #[test]
    fn connectives_and_homophones() {
        assert_eq!(disp("命令 加 西").as_deref(), Some("CMD+C")); // 连接词 + 谐音
        assert_eq!(disp("cmd加c").as_deref(), Some("CMD+C"));
        assert_eq!(disp("shift 和 command 与 e").as_deref(), Some("SHIFT+CMD+E"));
    }

    #[test]
    fn repeated_and_conflicting_keys() {
        assert_eq!(disp("command c c").as_deref(), Some("CMD+C")); // 重复键名去重
        assert!(parse("command c v", &builtin_lexicon()).is_none()); // 冲突键名放弃
    }

    #[test]
    fn unknown_ratio_guard() {
        assert!(parse("哒哒哒哒", &builtin_lexicon()).is_none());
        assert!(parse("一", &builtin_lexicon()).is_none()); // 裸数字无修饰词
        // unknown 过半时即使含别名也判废
        assert!(parse("哒哒哒复制哒哒哒哒哒哒", &builtin_lexicon()).is_none());
    }

    #[test]
    fn user_entries_take_priority() {
        use crate::config::{CommandConfig, CommandStopEntry};

        let user_cfg = CommandConfig {
            stop: vec![CommandStopEntry {
                phrase: "复制".into(),
            }],
            ..Default::default()
        };
        let lex = Lexicon::build(Some(&user_cfg));

        // "复制" 现在是停用词，所以不再产生 CMD+C
        assert!(super::parse("复制", &lex).is_none());
        // 内置的 "copy" 仍然有效
        assert_eq!(
            super::parse("copy", &lex).map(|c| c.display()).as_deref(),
            Some("CMD+C")
        );
    }

    #[test]
    fn user_action_appended() {
        use crate::config::{CommandActionEntry, CommandConfig};

        let user_cfg = CommandConfig {
            action: vec![CommandActionEntry {
                phrase: "截图".into(),
                modifiers: vec![Modifier::Shift, Modifier::Command],
                key: "4".into(),
            }],
            ..Default::default()
        };
        let lex = Lexicon::build(Some(&user_cfg));

        let result = super::parse("截图", &lex).map(|c| c.display());
        assert_eq!(result.as_deref(), Some("SHIFT+CMD+4"));
    }

    #[test]
    fn user_entry_does_not_break_builtin() {
        use crate::config::{CommandActionEntry, CommandConfig};

        let user_cfg = CommandConfig {
            action: vec![CommandActionEntry {
                phrase: "截图".into(),
                modifiers: vec![Modifier::Command],
                key: "3".into(),
            }],
            ..Default::default()
        };
        let lex = Lexicon::build(Some(&user_cfg));

        // 内置词表不受用户条目影响
        assert_eq!(
            super::parse("复制", &lex).map(|c| c.display()).as_deref(),
            Some("CMD+C")
        );
    }
}
