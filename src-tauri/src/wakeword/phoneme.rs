//! 关键词 → 音素 token（硬编码）。
//!
//! sherpa-onnx Zipformer KWS 模型只接受音素 token 格式的关键词
//! （如 `x iǎo y ì j ì @小易记`），不能直接传中文。
//!
//! 本模块硬编码五个内置唤醒词的音素序列，仅作参考；
//! 实际关键词管理已移至 `mod.rs` 的 `resolve_keywords()` 和 `create_engine()`。

use super::WakeWord;

// ── 硬编码唤醒词 ──────────────────────────────────────────────────────

/// 五个内置唤醒词及其音素表示（经 sherpa-onnx text2token 验证）。
const DEFAULT_ENTRIES: &[(&str, &str, &str)] = &[
    ("小易记", "x iǎo y ì j ì @小易记", "input"),
    ("小易修", "x iǎo y ì x iū @小易修", "repair"),
    ("小易控", "x iǎo y ì k òng @小易控", "command"),
    ("小易确认", "x iǎo y ì q uè r èn @小易确认", "commit"),
    ("小易清空", "x iǎo y ì q īng k ōng @小易清空", "clear"),
];

/// 获取硬编码的关键词 → WakeWord 映射。
pub fn default_keyword_map() -> Vec<(String, WakeWord)> {
    DEFAULT_ENTRIES
        .iter()
        .map(|(text, _, action)| {
            (
                text.to_string(),
                WakeWord {
                    text: text.to_string(),
                    action: action.to_string(),
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_keyword_map() {
        let map = default_keyword_map();
        assert_eq!(map.len(), 5);
        assert_eq!(map[0].0, "小易记");
        assert_eq!(map[1].0, "小易修");
        assert_eq!(map[2].0, "小易控");
        assert_eq!(map[3].0, "小易确认");
        assert_eq!(map[4].0, "小易清空");
    }
}
