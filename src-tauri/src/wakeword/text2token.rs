//! 自然语言关键词 → 音素 token 转换（text2token）。
//!
//! 参照 sherpa-onnx 官方 Python 实现（utils.py 的 `text2token()` 函数），
//! 支持 `phone+ppinyin` 模式：
//!
//! - 英文部分：查 en.phone（官方 126K 条 CMUdict 发音词典）；整词未命中时回退到字母逐拼
//! - 中文部分：查预计算 CJK→ppinyin 映射表获取声母+韵母（带声调）
//!
//! 使用方式：
//! ```ignore
//! let t2t = Text2Token::load(&model_dir)?;
//! let token_line = t2t.convert("小易记", "小易记")?;
//! // → "x iǎo y ì j ì @小易记"
//! ```
//!
//! 数据文件（需放在模型目录下）：
//! - `tokens.txt`          —— 模型的 token 词汇表
//! - `cjk_ppinyin.json`    —— CJK 字符 → ppinyin token 映射（由 pypinyin 预生成）
//! - `en.phone`            —— 英文发音词典（随 sherpa-onnx 模型附带）

use std::collections::HashMap;
use std::path::Path;

// ── 字母名发音（用于未知英文词的逐字母回退）──────────────────────────
//
// en.phone 中单字母 "A" 的读音是冠词 AH0 而非字母名 EY1。
// 为确保缩写/首字母缩略词的正确拼读，在此硬编码 26 个字母的标准
// CMUdict 字母名发音。

const LETTER_NAMES: &[(&str, &[&str])] = &[
    ("A", &["EY1"]),
    ("B", &["B", "IY1"]),
    ("C", &["S", "IY1"]),
    ("D", &["D", "IY1"]),
    ("E", &["IY1"]),
    ("F", &["EH1", "F"]),
    ("G", &["JH", "IY1"]),
    ("H", &["EY1", "CH"]),
    ("I", &["AY1"]),
    ("J", &["JH", "EY1"]),
    ("K", &["K", "EY1"]),
    ("L", &["EH1", "L"]),
    ("M", &["EH1", "M"]),
    ("N", &["EH1", "N"]),
    ("O", &["OW1"]),
    ("P", &["P", "IY1"]),
    ("Q", &["K", "Y", "UW1"]),
    ("R", &["AA1", "R"]),
    ("S", &["EH1", "S"]),
    ("T", &["T", "IY1"]),
    ("U", &["Y", "UW1"]),
    ("V", &["V", "IY1"]),
    ("W", &["D", "AH1", "B", "AH0", "L", "Y", "UW0"]),
    ("X", &["EH1", "K", "S"]),
    ("Y", &["W", "AY1"]),
    ("Z", &["Z", "IY1"]),
];

// ── Text2Token ───────────────────────────────────────────────────────────

/// 文本→音素 token 转换器。
///
/// 加载 tokens.txt、CJK 拼音映射表、英文 lexicon 三份数据，
/// 将自然语言关键词（如 "小易记"）转换为 sherpa-onnx KWS 模型
/// 所需的音素 token 格式（如 "x iǎo y ì j ì @小易记"）。
pub struct Text2Token {
    /// token 字符串 → token ID（从 tokens.txt 加载）
    token_table: HashMap<String, i32>,
    /// CJK 字符 → ppinyin token 列表（从 cjk_ppinyin.json 加载）
    cjk_map: HashMap<char, Vec<String>>,
    /// 英文单词（小写）→ ARPABET 音素列表（从 en.phone 加载）
    en_phone: HashMap<String, Vec<String>>,
}

impl Text2Token {
    // ── 加载 ──────────────────────────────────────────────────────────

    /// 从模型目录加载全部数据文件。
    ///
    /// 需要在目录中存在：
    /// - `tokens.txt`
    /// - `cjk_ppinyin.json`
    /// - `en.phone`
    pub fn load(model_dir: &Path) -> anyhow::Result<Self> {
        let token_table = Self::load_tokens(&model_dir.join("tokens.txt"))?;
        let cjk_map = Self::load_cjk_map(&model_dir.join("cjk_ppinyin.json"))?;
        let en_phone = Self::load_en_phone(&model_dir.join("en.phone"))?;

        eprintln!(
            "[drop-typing] text2token：已加载 {} tokens, {} CJK 字符, {} 英文词条 (en.phone)",
            token_table.len(),
            cjk_map.len(),
            en_phone.len(),
        );

        Ok(Self {
            token_table,
            cjk_map,
            en_phone,
        })
    }

    /// 加载 tokens.txt。
    ///
    /// 格式：每行 `<token> <id>`，用空白分隔。
    fn load_tokens(path: &Path) -> anyhow::Result<HashMap<String, i32>> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            anyhow::anyhow!("无法读取 tokens.txt '{}': {}", path.display(), e)
        })?;

        let mut map = HashMap::new();
        for (lineno, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() != 2 {
                anyhow::bail!(
                    "tokens.txt 第 {} 行格式错误（期望 '<token> <id>'，实际 {} 列）：{}",
                    lineno + 1,
                    parts.len(),
                    line,
                );
            }
            let id: i32 = parts[1].parse().map_err(|_| {
                anyhow::anyhow!(
                    "tokens.txt 第 {} 行 ID 不是整数：{}",
                    lineno + 1,
                    parts[1],
                )
            })?;
            map.insert(parts[0].to_string(), id);
        }

        if map.is_empty() {
            anyhow::bail!("tokens.txt 为空");
        }

        Ok(map)
    }

    /// 加载预计算的 CJK → ppinyin 映射表（JSON 格式）。
    ///
    /// 格式：`{"字": ["token1", "token2", ...], ...}`
    fn load_cjk_map(path: &Path) -> anyhow::Result<HashMap<char, Vec<String>>> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            anyhow::anyhow!(
                "无法读取 cjk_ppinyin.json '{}': {}",
                path.display(),
                e,
            )
        })?;

        let raw: HashMap<String, Vec<String>> =
            serde_json::from_str(&content).map_err(|e| {
                anyhow::anyhow!("cjk_ppinyin.json 解析失败: {}", e)
            })?;

        // 每个 key 应该恰好是一个字符，转换为 char → Vec<String>
        let map: HashMap<char, Vec<String>> = raw
            .into_iter()
            .filter_map(|(k, v)| {
                let ch = k.chars().next()?;
                if k.chars().count() == 1 {
                    Some((ch, v))
                } else {
                    None
                }
            })
            .collect();

        if map.is_empty() {
            anyhow::bail!("cjk_ppinyin.json 为空");
        }

        Ok(map)
    }

    /// 加载 en.phone（sherpa-onnx 官方英文发音词典，CMUdict 格式）。
    ///
    /// 格式：每行 `<单词> <音素1> <音素2> ...`，无注释行。
    /// 单词保留原始大小写，查表时统一按大写匹配。
    fn load_en_phone(path: &Path) -> anyhow::Result<HashMap<String, Vec<String>>> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            anyhow::anyhow!(
                "无法读取 en.phone '{}': {}",
                path.display(),
                e,
            )
        })?;

        let mut map = HashMap::new();
        for (lineno, line) in content.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 2 {
                anyhow::bail!(
                    "en.phone 第 {} 行格式错误（期望 '<word> <phone1> ...'）：{}",
                    lineno + 1,
                    line,
                );
            }
            // 用大写做 key（与 CMUdict 一致，大小写不敏感查表）
            let word = parts[0].to_uppercase();
            let phones: Vec<String> = parts[1..].iter().map(|s| s.to_string()).collect();
            map.insert(word, phones);
        }

        if map.is_empty() {
            anyhow::bail!("en.phone 为空");
        }

        Ok(map)
    }

    // ── 转换 ──────────────────────────────────────────────────────────

    /// 将一条自然语言关键词转换为音素 token 行。
    ///
    /// `keyword` 是用户写的自然语言关键词（如 `"小易记"`、`"小助手"`）。
    /// `label` 是检测结果标签（如 `"小易记"`），会以 `@label` 形式附在行尾。
    ///
    /// 返回可直接写入 keywords.txt 的一行文本。
    ///
    /// # 错误
    ///
    /// - 关键词中包含不在 CJK 映射表中的字符
    /// - 关键词中包含不在 lexicon 中的英文词
    /// - 最终 token 不在 tokens.txt 中
    pub fn convert(&self, keyword: &str, label: &str) -> anyhow::Result<String> {
        let segments = self.split_mixed_text(keyword);
        let mut tokens: Vec<String> = Vec::new();

        for seg in &segments {
            if seg.is_empty() {
                continue;
            }

            if self.is_single_cjk(seg) {
                // 单个 CJK 字符 → 查拼音映射表
                let ch = seg.chars().next().unwrap();
                let phonemes = self.cjk_map.get(&ch).ok_or_else(|| {
                    anyhow::anyhow!("CJK 字符 '{}' 没有拼音映射", ch)
                })?;

                for p in phonemes {
                    self.check_token(p, &format!("CJK 字符 '{}' 的拼音", ch))?;
                }
                tokens.extend(phonemes.clone());
            } else {
                // 非 CJK（英文词或缩写）→ 查 en.phone，未命中则逐字母回退
                let phonemes = self.lookup_english(seg)?;

                for p in phonemes.iter() {
                    self.check_token(p, &format!("英文段 '{}' 的音素", seg))?;
                }
                tokens.extend(phonemes);
            }
        }

        if tokens.is_empty() {
            anyhow::bail!("关键词 '{}' 转换后没有有效 token", keyword);
        }

        // 附加 @label（sherpa-onnx 的 keyword 标签格式）
        tokens.push(format!("@{}", label));

        Ok(tokens.join(" "))
    }

    /// 批量转换。每项为 `(keyword, label)`。
    pub fn convert_batch(
        &self,
        items: &[(String, String)],
    ) -> anyhow::Result<Vec<String>> {
        items
            .iter()
            .map(|(kw, label)| self.convert(kw, label))
            .collect()
    }

    /// 批量转换并写入 keywords.txt 文件。
    pub fn write_keywords_txt(
        &self,
        items: &[(String, String)],
        output_path: &Path,
    ) -> anyhow::Result<()> {
        let lines = self.convert_batch(items)?;
        let content = lines.join("\n") + "\n";
        std::fs::write(output_path, &content).map_err(|e| {
            anyhow::anyhow!(
                "无法写入 keywords.txt '{}': {}",
                output_path.display(),
                e,
            )
        })?;
        eprintln!(
            "[drop-typing] text2token：已写入 {} 条关键词到 '{}'",
            items.len(),
            output_path.display(),
        );
        Ok(())
    }

    // ── 内部辅助 ──────────────────────────────────────────────────────

    /// 检查 token 是否在 token_table 中存在。
    fn check_token(&self, token: &str, context: &str) -> anyhow::Result<()> {
        if !self.token_table.contains_key(token) {
            anyhow::bail!(
                "{} '{}' 不在 tokens.txt 中，请确认模型 token 词汇表是否匹配",
                context,
                token,
            );
        }
        Ok(())
    }

    /// 英文段 → ARPABET 音素序列。
    ///
    /// 先用整词查 en.phone，未命中则回退到逐字母拼读。
    fn lookup_english(&self, text: &str) -> anyhow::Result<Vec<String>> {
        let upper = text.to_uppercase();

        // 1. 整词在 en.phone 中 → 直接返回
        if let Some(phones) = self.en_phone.get(&upper) {
            return Ok(phones.clone());
        }

        // 2. 逐字母回退（用于缩写/首字母缩略词如 "DT"、"ASR" 等）
        let mut result = Vec::new();
        for ch in text.chars() {
            if !ch.is_ascii_alphabetic() {
                anyhow::bail!(
                    "英文段 '{}' 不在 en.phone 中，且包含非字母字符 '{}'，无法逐字母拼读",
                    text,
                    ch,
                );
            }
            let letter = ch.to_uppercase().to_string();
            let phones = LETTER_NAMES
                .iter()
                .find(|(l, _)| *l == letter)
                .map(|(_, p)| *p)
                .ok_or_else(|| {
                    anyhow::anyhow!("无法找到字母 '{}' 的发音", letter)
                })?;
            result.extend(phones.iter().map(|s| s.to_string()));
        }

        Ok(result)
    }

    /// 判断字符串是否为单个 CJK 字符。
    fn is_single_cjk(&self, s: &str) -> bool {
        let mut chars = s.chars();
        match chars.next() {
            Some(ch) if chars.next().is_none() => is_cjk_char(ch),
            _ => false,
        }
    }

    /// 按 CJK 字符边界拆分混合文本。
    ///
    /// ```text
    /// "小易记"     → ["小", "易", "记"]
    /// "你好 DT"    → ["你", "好", "DT"]
    /// "小爱同学"   → ["小", "爱", "同", "学"]
    /// ```
    ///
    /// 对标 Python 实现的
    /// `re.compile(r"([一-鿿])").split(text)`。
    fn split_mixed_text(&self, text: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut current = String::new();

        for ch in text.chars() {
            if is_cjk_char(ch) {
                // 遇到 CJK 字符：先 flush 累积的英文段
                if !current.is_empty() {
                    result.push(std::mem::take(&mut current));
                }
                // 每个 CJK 字符单独一段
                result.push(ch.to_string());
            } else if ch.is_whitespace() {
                // 空白视为段分隔符
                if !current.is_empty() {
                    result.push(std::mem::take(&mut current));
                }
            } else {
                // 非 CJK、非空白字符（英文字母、数字、标点等）累积
                current.push(ch);
            }
        }
        // flush 剩余
        if !current.is_empty() {
            result.push(current);
        }

        result
    }
}

// ── CJK 字符检测 ─────────────────────────────────────────────────────────

/// 判断字符是否在 CJK 统一表意文字范围内。
///
/// 覆盖：
/// - CJK 基本区：U+4E00..U+9FFF
/// - CJK 扩展 A：U+3400..U+4DBF
/// - CJK 兼容区：U+F900..U+FAFF
fn is_cjk_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{4E00}'..='\u{9FFF}'
        | '\u{3400}'..='\u{4DBF}'
        | '\u{F900}'..='\u{FAFF}'
    )
}

// ── 测试 ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// 获取内置模型目录路径（开发模式下的回退逻辑）
    fn builtin_model_dir() -> PathBuf {
        let dirs = [
            // CARGO_MANIFEST_DIR
            option_env!("CARGO_MANIFEST_DIR").map(|d| {
                PathBuf::from(d).join("models/builtin/sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20")
            }),
            // 相对路径
            Some(PathBuf::from(
                "models/builtin/sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20",
            )),
        ];

        for d in dirs.into_iter().flatten() {
            if d.is_dir() {
                return d;
            }
        }
        panic!("找不到内置模型目录");
    }

    #[test]
    fn test_split_mixed_text() {
        let model_dir = builtin_model_dir();
        let t2t = Text2Token::load(&model_dir).expect("加载 text2token");

        assert_eq!(t2t.split_mixed_text("DT打"), vec!["DT", "打"]);
        assert_eq!(t2t.split_mixed_text("DT修"), vec!["DT", "修"]);
        assert_eq!(t2t.split_mixed_text("DT控"), vec!["DT", "控"]);
        assert_eq!(
            t2t.split_mixed_text("小爱同学"),
            vec!["小", "爱", "同", "学"]
        );
        assert_eq!(
            t2t.split_mixed_text("你好 DT"),
            vec!["你", "好", "DT"]
        );
        assert_eq!(
            t2t.split_mixed_text("HELLO 世界"),
            vec!["HELLO", "世", "界"]
        );
        assert_eq!(t2t.split_mixed_text("OK"), vec!["OK"]);
        assert_eq!(t2t.split_mixed_text(""), Vec::<String>::new());
    }

    #[test]
    fn test_convert_builtin_keywords() {
        let model_dir = builtin_model_dir();
        let t2t = Text2Token::load(&model_dir).expect("加载 text2token");

        // 这三个是 phoneme.rs 中硬编码的唤醒词，必须与 keywords.txt 一致
        let result_da = t2t.convert("DT打", "DT打").expect("转换 DT打");
        assert_eq!(result_da, "D IY1 T IY1 d ǎ @DT打");

        let result_xiu = t2t.convert("DT修", "DT修").expect("转换 DT修");
        assert_eq!(result_xiu, "D IY1 T IY1 x iū @DT修");

        let result_kong = t2t.convert("DT控", "DT控").expect("转换 DT控");
        assert_eq!(result_kong, "D IY1 T IY1 k òng @DT控");
    }

    #[test]
    fn test_convert_pure_chinese() {
        let model_dir = builtin_model_dir();
        let t2t = Text2Token::load(&model_dir).expect("加载 text2token");

        let result_xy = t2t.convert("小易记", "小易记").expect("转换 小易记");
        assert_eq!(result_xy, "x iǎo y ì j ì @小易记");

        let result = t2t.convert("小助手", "小助手").expect("转换 小助手");
        assert!(result.starts_with("x iǎo zh ù sh ǒu"));
        assert!(result.ends_with("@小助手"));

        let result2 = t2t.convert("你好", "你好").expect("转换 你好");
        assert!(result2.starts_with("n ǐ h ǎo"));
        assert!(result2.ends_with("@你好"));
    }

    #[test]
    fn test_convert_mixed() {
        let model_dir = builtin_model_dir();
        let t2t = Text2Token::load(&model_dir).expect("加载 text2token");

        let result = t2t.convert("你好 DT", "你好_DT").expect("转换 你好 DT");
        // "你好 DT" → 你: n ǐ, 好: h ǎo, DT: D IY1 T IY1
        assert!(result.contains("n ǐ"));
        assert!(result.contains("h ǎo"));
        assert!(result.contains("D IY1 T IY1"));
        assert!(result.ends_with("@你好_DT"));
    }

    #[test]
    fn test_letter_by_letter_fallback() {
        let model_dir = builtin_model_dir();
        let t2t = Text2Token::load(&model_dir).expect("加载 text2token");

        // "DT" 不在 en.phone 中，应回退到逐字母 D→D IY1, T→T IY1
        let result = t2t.convert("DT打", "DT打").expect("转换 DT打");
        assert!(result.contains("D IY1 T IY1"), "应包含 D IY1 T IY1: {}", result);

        // "ASR" 不在 en.phone 中，回退逐字母
        let result2 = t2t.convert("ASR测试", "ASR测试").expect("转换 ASR测试");
        assert!(result2.contains("EY1 EH1 S AA1 R"), "应逐字母拼读 ASR: {}", result2);
        assert!(result2.contains("c è sh ì"), "应含中文部分: {}", result2);
    }

    #[test]
    fn test_non_alpha_english_rejected() {
        let model_dir = builtin_model_dir();
        let t2t = Text2Token::load(&model_dir).expect("加载 text2token");

        // 英文段含数字 → 字母逐拼失败，应报错
        let err = t2t.convert("HELLO3打", "test").unwrap_err();
        assert!(
            err.to_string().contains("3"),
            "错误信息应包含非字母字符：{}",
            err,
        );
    }

    #[test]
    fn test_write_keywords_txt() {
        let model_dir = builtin_model_dir();
        let t2t = Text2Token::load(&model_dir).expect("加载 text2token");

        let tmp = std::env::temp_dir().join("test_keywords_drop_typing.txt");
        let items: Vec<(String, String)> = vec![
            ("小易记".into(), "小易记".into()),
            ("小易修".into(), "小易修".into()),
            ("小易小易".into(), "小易小易".into()),
            ("小易确认".into(), "小易确认".into()),
            ("小易清空".into(), "小易清空".into()),
        ];

        t2t.write_keywords_txt(&items, &tmp).expect("写入 keywords.txt");

        let content = std::fs::read_to_string(&tmp).expect("读取 keywords.txt");
        let lines: Vec<&str> = content.trim().lines().collect();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0], "x iǎo y ì j ì @小易记");
        assert_eq!(lines[1], "x iǎo y ì x iū @小易修");
        assert_eq!(lines[2], "x iǎo y ì x iǎo y ì @小易小易");
        assert_eq!(lines[3], "x iǎo y ì q uè r èn @小易确认");
        assert_eq!(lines[4], "x iǎo y ì q īng k ōng @小易清空");

        // 清理
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_convert_xiaoyi_builtin_keywords() {
        let model_dir = builtin_model_dir();
        let t2t = Text2Token::load(&model_dir).expect("加载 text2token");
        let expected = [
            ("小易记", "x iǎo y ì j ì @小易记"),
            ("小易修", "x iǎo y ì x iū @小易修"),
            ("小易小易", "x iǎo y ì x iǎo y ì @小易小易"),
            ("小易确认", "x iǎo y ì q uè r èn @小易确认"),
            ("小易清空", "x iǎo y ì q īng k ōng @小易清空"),
        ];
        for (keyword, expected_line) in expected {
            let line = t2t.convert(keyword, keyword).expect("转换");
            assert_eq!(line, expected_line, "小易小易 音素应与模型转换一致，实际：{line}");
        }
    }
}
