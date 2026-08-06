//! 唤醒词引擎抽象。
//!
//! 使用 sherpa-onnx KeywordSpotter 检测唤醒词。支持：
//! - 内置默认唤醒词（小易记/小易修/小易控/小易确认/小易清空），通过 text2token 动态生成
//! - 用户自定义唤醒词（配置文件中的 [[wakeword.keywords]]），通过 text2token 动态生成
//!
//! 启动逻辑：
//! 1. 检查用户配置中是否有自定义唤醒词
//! 2. 有 → 用 text2token 动态生成 token buf → 以此加载 KeywordSpotter
//! 3. 无 → 从静态 keywords.txt 加载（向后兼容）

pub mod phoneme;
pub mod sherpa;
pub mod text2token;

use std::path::Path;

use crate::config::KeywordEntry;

// ── 类型定义 ─────────────────────────────────────────────────────────

/// 一个唤醒词，包含关键词文本和对应动作。
///
/// 从配置文件或内置默认值构建。`action` 决定检测后进入的通道：
/// - `"input"`   → 录音 → ASR → 追加到暂存条
/// - `"repair"`  → 录音 → ASR → 替换暂存条内容
/// - `"command"` → 录音 → ASR → 解析为指令执行
/// - `"commit"`  → 立即提交暂存条到光标处（不录音）
/// - `"clear"`   → 立即清空暂存条并隐藏（不录音）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WakeWord {
    /// 自然语言关键词文本（如 "小易记"）
    pub text: String,
    /// 检测后进入的通道："input" | "repair" | "command"
    pub action: String,
}

impl WakeWord {
    /// 唤醒词自身估计时长（毫秒），用于裁切 RingBuffer。
    ///
    /// 粗略按中文 250ms/字 + 英文 150ms/字母 估算。
    pub fn duration_ms(&self) -> u64 {
        let mut ms = 0u64;
        for ch in self.text.chars() {
            if ('\u{4E00}'..='\u{9FFF}').contains(&ch) {
                ms += 250;
            } else if ch.is_ascii_alphabetic() {
                ms += 150;
            } else {
                ms += 100;
            }
        }
        ms.clamp(300, 1200)
    }

    /// 显示名（暂存条状态徽章用）。
    pub fn display_name(&self) -> &str {
        &self.text
    }
}

// ── WakeEvent ─────────────────────────────────────────────────────────

/// 从唤醒词引擎发出的事件。
#[derive(Debug, Clone)]
pub enum WakeEvent {
    /// 检测到唤醒词。`position` 是 RingBuffer 中唤醒词结束时的绝对采样位置。
    Detected {
        word: WakeWord,
        /// 唤醒词结束位置（RingBuffer 绝对采样序号），
        /// pipeline 据此定位裁切点
        position: u64,
    },
}

// ── 内置默认值 ─────────────────────────────────────────────────────────

/// 三个内置默认唤醒词（用户未自定义时使用）。
const BUILTIN_DEFAULTS: &[(&str, &str)] = &[
    ("小易记", "input"),
    ("小易修", "repair"),
    ("小易控", "command"),
    ("小易确认", "commit"),
    ("小易清空", "clear"),
];

// ── 关键词列表构建 ─────────────────────────────────────────────────────

/// 根据配置决定关键词列表。
///
/// 返回 `(keyword_map, use_dynamic)`：
/// - `keyword_map`：`[(keyword_text, WakeWord)]` 列表，供 SherpaKws 加载
/// - `use_dynamic`：`true` 表示需要 text2token 动态生成 token，`false` 表示用静态 keywords.txt
pub fn resolve_keywords(cfg: &crate::config::WakewordConfig) -> (Vec<(String, WakeWord)>, bool) {
    if !cfg.keywords.is_empty() {
        let map: Vec<(String, WakeWord)> = cfg
            .keywords
            .iter()
            .map(|e: &KeywordEntry| {
                (
                    e.keyword.clone(),
                    WakeWord {
                        text: e.keyword.clone(),
                        action: e.action.clone(),
                    },
                )
            })
            .collect();
        (map, true)
    } else {
        let map: Vec<(String, WakeWord)> = BUILTIN_DEFAULTS
            .iter()
            .map(|(text, action)| {
                (
                    text.to_string(),
                    WakeWord {
                        text: text.to_string(),
                        action: action.to_string(),
                    },
                )
            })
            .collect();
        // 内置默认值也走 text2token 动态路径（含小易确认/小易清空 等）
        (map, true)
    }
}

// ── 工厂 ─────────────────────────────────────────────────────────────

/// 尝试创建 sherpa-onnx 唤醒词引擎。
///
/// 流程：
/// 1. 解析模型目录
/// 2. 根据用户配置决定关键词来源（动态生成 vs 静态文件）
/// 3. 加载 KeywordSpotter
///
/// 加载失败时返回 `None`（优雅降级，热键仍然可用）。
pub fn create_engine(
    cfg: &crate::config::WakewordConfig,
    resource_dir: &Path,
) -> Option<sherpa::SherpaKws> {
    let model_dir = sherpa::resolve_model_dir(&cfg.model_dir, resource_dir);

    let model_dir = match model_dir {
        Some(d) => d,
        None => {
            eprintln!(
                "[drop-typing] 唤醒词：模型目录未找到 '{}'",
                cfg.model_dir,
            );
            return None;
        }
    };

    let (keyword_map, use_dynamic) = resolve_keywords(cfg);

    if use_dynamic {
        // 用户自定义关键词 → text2token 动态生成 token buf
        let t2t = match text2token::Text2Token::load(&model_dir) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[drop-typing] 唤醒词：text2token 加载失败：{e}");
                return None;
            }
        };

        let items: Vec<(String, String)> = keyword_map
            .iter()
            .map(|(text, ww)| (text.clone(), ww.text.clone()))
            .collect();

        match t2t.convert_batch(&items) {
            Ok(lines) => {
                let keywords_buf = lines.join("\n");
                eprintln!(
                    "[drop-typing] 唤醒词：从用户配置动态生成 {} 个关键词",
                    items.len(),
                );
                for (i, line) in lines.iter().enumerate() {
                    eprintln!("[drop-typing]   [{i}] {line}");
                }
                match sherpa::SherpaKws::load_with_buf(
                    &model_dir,
                    &keyword_map,
                    &keywords_buf,
                    cfg.keywords_threshold,
                    cfg.keywords_score,
                ) {
                    Ok(engine) => Some(engine),
                    Err(e) => {
                        eprintln!("[drop-typing] 唤醒词：动态加载失败：{e}");
                        None
                    }
                }
            }
            Err(e) => {
                eprintln!("[drop-typing] 唤醒词：text2token 转换失败：{e}");
                None
            }
        }
    } else {
        eprintln!(
            "[drop-typing] 唤醒词：从默认 keywords.txt 加载 {} 个关键词",
            keyword_map.len(),
        );
        match sherpa::SherpaKws::load(
            &model_dir,
            &keyword_map,
            cfg.keywords_threshold,
            cfg.keywords_score,
        ) {
            Ok(engine) => Some(engine),
            Err(e) => {
                eprintln!("[drop-typing] 唤醒词：加载失败：{e}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_defaults_are_xiaoyi_words() {
        let texts: Vec<&str> = BUILTIN_DEFAULTS.iter().map(|(t, _)| *t).collect();
        assert_eq!(
            texts,
            vec!["小易记", "小易修", "小易控", "小易确认", "小易清空"]
        );
        let actions: Vec<&str> = BUILTIN_DEFAULTS.iter().map(|(_, a)| *a).collect();
        assert_eq!(actions, vec!["input", "repair", "command", "commit", "clear"]);
    }
}
