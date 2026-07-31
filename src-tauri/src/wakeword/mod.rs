//! 唤醒词引擎抽象。
//!
//! 使用 sherpa-onnx KeywordSpotter 检测三个内置中文唤醒词（硬编码，不支持自定义）：
//!
//! | 唤醒词 | 通道   | 行为 |
//! |--------|--------|------|
//! | DT打   | Input  | 说话 → ASR → LLM 清洗 → 追加 |
//! | DT修   | Repair | 说修正指令 → LLM repair → 替换 |
//! | DT控   | Command| 说按键名 → 本地解析 → 模拟按键 |
//!
//! 启动时直接读模型目录下的 keywords.txt（静态文件），不做动态生成。

pub mod phoneme;
pub mod sherpa;

use std::path::Path;

// ── 类型定义 ─────────────────────────────────────────────────────────

/// 三个唤醒词。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeWord {
    /// "DT打" → 录入通道
    Da,
    /// "DT修" → 修复通道
    Xiu,
    /// "DT控" → 指令通道
    An,
}

impl WakeWord {
    /// 唤醒词自身估计时长（毫秒），用于裁切 RingBuffer。
    pub fn duration_ms(self) -> u64 {
        match self {
            WakeWord::Da => 500,
            WakeWord::Xiu => 550,
            WakeWord::An => 450,
        }
    }

    /// 显示名（暂存条状态徽章用）。
    pub fn display_name(self) -> &'static str {
        match self {
            WakeWord::Da => "DT打",
            WakeWord::Xiu => "DT修",
            WakeWord::An => "DT控",
        }
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

// ── 工厂 ─────────────────────────────────────────────────────────────

/// 尝试创建 sherpa-onnx 唤醒词引擎。
///
/// 流程：
/// 1. 解析模型目录
/// 2. 写入 keywords.txt（仅当文件缺失时，使用硬编码默认值）
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

    // 硬编码关键词：DT打/DT修/DT控
    let keyword_map = phoneme::default_keyword_map();

    // 加载模型（keywords.txt 已预置在模型目录中）
    match sherpa::SherpaKws::load(
        &model_dir,
        &keyword_map,
        cfg.keywords_threshold,
        cfg.keywords_score,
    ) {
        Ok(engine) => {
            eprintln!(
                "[drop-typing] 唤醒词（sherpa-onnx）：已加载 {} 个关键词（DT打/DT修/DT控）",
                keyword_map.len(),
            );
            Some(engine)
        }
        Err(e) => {
            eprintln!("[drop-typing] 唤醒词：加载失败：{e}");
            None
        }
    }
}
