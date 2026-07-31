//! sherpa-onnx KeywordSpotter 封装。
//!
//! 封装 sherpa-onnx 的流式 Keyword Spotter API：
//! - 加载 Zipformer Transducer KWS 模型（encoder/decoder/joiner + tokens.txt）
//! - 提供 `process_frame()` 供唤醒词线程每 80ms 调用
//! - 管理 stream 生命周期：创建、喂音频、解码、检测、重置
//!
//! 模型目录结构（sherpa-onnx 预训练 KWS 模型）：
//! ```text
//! {model_dir}/
//!   encoder.onnx
//!   decoder.onnx
//!   joiner.onnx
//!   tokens.txt
//!   keywords.txt    ← 我们用 token 格式写入的唤醒词文件
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use sherpa_onnx::{KeywordSpotter, KeywordSpotterConfig, KeywordResult};

use super::WakeWord;

// ── 模型路径解析 ──────────────────────────────────────────────────────

/// 解析模型目录路径。
///
/// `resource_dir` 是 Tauri 的资源根目录（`app.path().resource_dir()`）。
/// - 以 `/` `./` `../` `~` 开头 → 视为文件系统路径
/// - 否则 → 在 `{resource_dir}/models/builtin/{model_dir}/` 查找
pub(crate) fn resolve_model_dir(model_dir: &str, resource_dir: &Path) -> Option<PathBuf> {
    let looks_like_path = model_dir.starts_with('/')
        || model_dir.starts_with("./")
        || model_dir.starts_with("../")
        || model_dir.starts_with('~');

    if looks_like_path {
        let p = Path::new(model_dir);
        if p.is_dir() {
            return Some(p.to_path_buf());
        }
        eprintln!(
            "[drop-typing] 唤醒词：指定的模型目录不存在 '{}'",
            model_dir,
        );
        return None;
    }

    // 内置模型目录：{resource_dir}/models/builtin/{model_dir}
    let builtin = resource_dir.join("models/builtin").join(model_dir);
    eprintln!(
        "[drop-typing] 唤醒词：查找模型目录 '{}' (resource_dir={})",
        builtin.display(),
        resource_dir.display(),
    );
    if builtin.is_dir() {
        return Some(builtin);
    }

    // 回退查找：尝试 CARGO_MANIFEST_DIR 下的 models/builtin
    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR") {
        let cm = Path::new(manifest_dir)
            .join("models")
            .join("builtin")
            .join(model_dir);
        eprintln!("[drop-typing] 唤醒词：尝试 CARGO_MANIFEST_DIR '{}'", cm.display());
        if cm.is_dir() {
            return Some(cm);
        }
    }

    // 回退：相对路径 models/builtin（dev 模式下当前工作目录可能是项目根）
    let rel = Path::new("models")
        .join("builtin")
        .join(model_dir);
    eprintln!("[drop-typing] 唤醒词：尝试相对路径 '{}'", rel.display());
    if rel.is_dir() {
        return Some(rel);
    }

    None
}

// ── SherpaKws ─────────────────────────────────────────────────────────

/// sherpa-onnx 唤醒词检测器。
///
/// 持有 `KeywordSpotter`（模型）和 keyword → WakeWord 映射表。
/// 线程安全：`KeywordSpotter` 实现了 `Send + Sync`。
pub struct SherpaKws {
    spotter: KeywordSpotter,
    /// 检测到的 keyword 字符串 → WakeWord 枚举
    keyword_map: HashMap<String, WakeWord>,
}

impl SherpaKws {
    /// 从模型目录加载唤醒词引擎。
    ///
    /// `keywords` 为 `[(keyword_text, wake_word), ...]` 列表，
    /// 内部调用 sherpa-onnx 的 tokenizer API 生成 token 格式的 keywords 字符串。
    pub fn load(
        model_dir: &Path,
        keywords: &[(String, WakeWord)],
        threshold: f32,
        score: f32,
    ) -> anyhow::Result<Self> {
        // 验证模型文件存在（keywords.txt 由调用方在之前生成）
        let encoder = model_dir.join("encoder.onnx");
        let decoder = model_dir.join("decoder.onnx");
        let joiner = model_dir.join("joiner.onnx");
        let tokens = model_dir.join("tokens.txt");
        let keywords_file = model_dir.join("keywords.txt");

        for (name, path) in [
            ("encoder.onnx", &encoder),
            ("decoder.onnx", &decoder),
            ("joiner.onnx", &joiner),
            ("tokens.txt", &tokens),
        ] {
            if !path.exists() {
                return Err(anyhow::anyhow!(
                    "模型文件缺失：{}（{name}）",
                    path.display()
                ));
            }
        }

        // keywords.txt 应由 phoneme::write_keywords_txt 在此前生成
        if !keywords_file.exists() {
            return Err(anyhow::anyhow!(
                "keywords.txt 缺失：{}",
                keywords_file.display(),
            ));
        }

        // 构建 keyword（@标签）→ WakeWord 映射表
        let keyword_map: HashMap<String, WakeWord> = keywords
            .iter()
            .map(|(k, w)| (k.clone(), *w))
            .collect();

        eprintln!(
            "[drop-typing] 唤醒词：关键词映射 {:#?}",
            keyword_map.keys().collect::<Vec<_>>(),
        );

        // 配置 KeywordSpotter —— 使用预生成的 keywords.txt（音素格式）
        let mut config = KeywordSpotterConfig::default();
        config.model_config.transducer.encoder = Some(
            encoder.to_str().unwrap().to_string(),
        );
        config.model_config.transducer.decoder = Some(
            decoder.to_str().unwrap().to_string(),
        );
        config.model_config.transducer.joiner = Some(
            joiner.to_str().unwrap().to_string(),
        );
        config.model_config.tokens = Some(
            tokens.to_str().unwrap().to_string(),
        );
        config.keywords_file = Some(
            keywords_file.to_str().unwrap().to_string(),
        );
        config.keywords_threshold = threshold;
        config.keywords_score = score;

        let spotter = KeywordSpotter::create(&config)
            .ok_or_else(|| anyhow::anyhow!("创建 KeywordSpotter 失败：模型加载返回 None"))?;

        eprintln!(
            "[drop-typing] 唤醒词（sherpa-onnx）：已加载模型 {}（{} 个关键词，阈值 {:.2}）",
            model_dir.display(),
            keywords.len(),
            threshold,
        );

        Ok(Self {
            spotter,
            keyword_map,
        })
    }

    /// 处理一帧音频（f32, 16kHz, 单声道）。
    ///
    /// 返回 `Some(WakeWord)` 如果检测到唤醒词，否则 `None`。
    /// 检测到后会自动重置 stream 以避免重复触发。
    pub fn process_frame(
        &self,
        stream: &mut sherpa_onnx::OnlineStream,
        frame: &[f32],
    ) -> Option<WakeWord> {
        // 喂音频
        stream.accept_waveform(16_000, frame);

        // 增量解码（带迭代上限以防死循环）
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

        // 检查结果
        let result: Option<KeywordResult> = self.spotter.get_result(stream);

        match result {
            Some(r) if !r.keyword.is_empty() => {
                // 在 keyword_map 中查找匹配
                // 注意：r.keyword 可能和输入不完全一致（如去掉了空格），
                // 做一次规范化比较
                let detected = r.keyword.trim().to_lowercase();
                let wake_word = self.keyword_map
                    .iter()
                    .find(|(k, _)| {
                        let normalized = k.trim().to_lowercase();
                        detected.contains(&normalized) || normalized.contains(&detected)
                    })
                    .map(|(_, w)| *w);

                eprintln!(
                    "[drop-typing] 🎤 检测到唤醒词 '{}'（置信度 start_time={:.3}s）",
                    r.keyword, r.start_time,
                );

                // 重置 stream，避免连续触发同一唤醒词
                self.spotter.reset(stream);

                wake_word
            }
            _ => None,
        }
    }

    /// 重置 stream 状态（外部调用，如检测后清除状态）。
    pub fn reset(&self, stream: &mut sherpa_onnx::OnlineStream) {
        self.spotter.reset(stream);
    }

    /// 创建新的 stream（用于独立检测会话）。
    pub fn create_stream(&self) -> sherpa_onnx::OnlineStream {
        self.spotter.create_stream()
    }
}

#[cfg(test)]
mod tests {
    // 关键词音素格式在 keywords.txt 中手动维护，不在此测试
}
