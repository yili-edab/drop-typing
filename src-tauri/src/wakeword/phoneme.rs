//! 关键词 → 音素 token（硬编码）。
//!
//! sherpa-onnx Zipformer KWS 模型只接受音素 token 格式的关键词
//! （如 `D IY1 T IY1 d ǎ @DT打`），不能直接传中文。
//!
//! 本模块硬编码三个内置唤醒词的音素序列，启动时写入 keywords.txt
//! 供 sherpa-onnx 加载。不支持用户自定义唤醒词。

use super::WakeWord;

// ── 硬编码唤醒词 ──────────────────────────────────────────────────────

/// 三个内置唤醒词及其音素表示（经 sherpa-onnx text2token 验证）。
const DEFAULT_ENTRIES: &[(&str, &str, WakeWord)] = &[
    ("DT打", "D IY1 T IY1 d ǎ @DT打", WakeWord::Da),
    ("DT修", "D IY1 T IY1 x iū @DT修", WakeWord::Xiu),
    ("DT控", "D IY1 T IY1 k òng @DT控", WakeWord::An),
];

/// 获取硬编码的关键词 → WakeWord 映射。
pub fn default_keyword_map() -> Vec<(String, WakeWord)> {
    DEFAULT_ENTRIES
        .iter()
        .map(|(text, _, word)| (text.to_string(), *word))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_keyword_map() {
        let map = default_keyword_map();
        assert_eq!(map.len(), 3);
        assert_eq!(map[0].0, "DT打");
        assert_eq!(map[1].0, "DT修");
        assert_eq!(map[2].0, "DT控");
    }
}
