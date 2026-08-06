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

/// 在多个基目录下依次查找 `models/builtin/{model_dir}`。
fn find_in_dirs(model_dir: &str, bases: &[&Path]) -> Option<PathBuf> {
    bases
        .iter()
        .map(|base| base.join("models").join("builtin").join(model_dir))
        .find(|p| p.is_dir())
}

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

    // 回退：可执行文件旁的 models/builtin（Windows 裸 exe / 便携部署）
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            eprintln!(
                "[drop-typing] 唤醒词：尝试 exe 同目录 '{}'",
                exe_dir.display(),
            );
            if let Some(p) = find_in_dirs(model_dir, &[exe_dir]) {
                return Some(p);
            }
        }
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
    /// 从模型目录加载唤醒词引擎（使用静态 keywords.txt 文件）。
    pub fn load(
        model_dir: &Path,
        keywords: &[(String, WakeWord)],
        threshold: f32,
        score: f32,
    ) -> anyhow::Result<Self> {
        let keywords_file = model_dir.join("keywords.txt");
        Self::load_impl(model_dir, keywords, None, Some(&keywords_file), threshold, score)
    }

    /// 从模型目录加载唤醒词引擎（使用动态生成的 token buffer）。
    ///
    /// `keywords_buf` 为 text2token 输出的 token 格式字符串（每行一个关键词）。
    /// 使用此方法时不需要 keywords.txt 文件。
    pub fn load_with_buf(
        model_dir: &Path,
        keywords: &[(String, WakeWord)],
        keywords_buf: &str,
        threshold: f32,
        score: f32,
    ) -> anyhow::Result<Self> {
        Self::load_impl(model_dir, keywords, Some(keywords_buf), None, threshold, score)
    }

    /// 统一的内部加载逻辑。
    ///
    /// `keywords_buf` 和 `keywords_file` 至少提供一个。
    fn load_impl(
        model_dir: &Path,
        keywords: &[(String, WakeWord)],
        keywords_buf: Option<&str>,
        keywords_file: Option<&Path>,
        threshold: f32,
        score: f32,
    ) -> anyhow::Result<Self> {
        // 验证模型文件存在
        let encoder = model_dir.join("encoder.onnx");
        let decoder = model_dir.join("decoder.onnx");
        let joiner = model_dir.join("joiner.onnx");
        let tokens = model_dir.join("tokens.txt");

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

        // keywords.txt 仅在未提供 buf 时需要
        if keywords_buf.is_none() {
            let kf = keywords_file.ok_or_else(|| {
                anyhow::anyhow!("keywords_buf 和 keywords_file 均为 None")
            })?;
            if !kf.exists() {
                return Err(anyhow::anyhow!("keywords.txt 缺失：{}", kf.display()));
            }
        }

        // 构建 keyword（@标签）→ WakeWord 映射表
        let keyword_map: HashMap<String, WakeWord> = keywords
            .iter()
            .map(|(k, w)| (k.clone(), w.clone()))
            .collect();

        // 配置 KeywordSpotter
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
        config.keywords_threshold = threshold;
        config.keywords_score = score;

        if let Some(buf) = keywords_buf {
            config.keywords_buf = Some(buf.to_string());
            eprintln!("[drop-typing] 唤醒词（sherpa-onnx）：使用动态 token buffer（{} 字节）", buf.len());
        } else if let Some(kf) = keywords_file {
            config.keywords_file = Some(kf.to_str().unwrap().to_string());
        }

        let spotter = KeywordSpotter::create(&config)
            .ok_or_else(|| anyhow::anyhow!("创建 KeywordSpotter 失败：模型加载返回 None"))?;

        eprintln!(
            "[drop-typing] 唤醒词（sherpa-onnx）：模型 {}（{} 个关键词，阈值 {:.2}）",
            model_dir.display(),
            keywords.len(),
            threshold,
        );

        Ok(Self {
            spotter,
            keyword_map,
        })
    }

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

    /// 处理一帧音频（f32, 16kHz, 单声道）。
    ///
    /// 返回 `Some(WakeWord)` 如果检测到唤醒词，否则 `None`。
    /// 检测到后会自动重置 stream 以避免重复触发。
    pub fn process_frame(
        &self,
        stream: &mut sherpa_onnx::OnlineStream,
        frame: &[f32],
    ) -> Option<WakeWord> {
        let detected = self.process_frame_label(stream, frame)?;
        // 在 keyword_map 中查找匹配。
        //
        // 匹配策略（按优先级）：
        // 1. 精确匹配（trim + 大小写归一化后相等）
        // 2. 前向包含：detected 包含 normalized（如 sherpa-onnx 返回
        //    "DT打" 带多余空格，配置里是 "DT打"）
        // 3. 后向包含：normalized 包含 detected（仅在前两项都未命中时回退，
        //    用于兼容旧逻辑，但不作为首选以避免 "杨力" 误匹配到 "杨力确认"）

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
    use super::*;

    // 关键词音素格式在 keywords.txt 中手动维护，不在此测试

    #[test]
    fn find_in_dirs_locates_model_dir() {
        let root = std::env::temp_dir().join(format!(
            "drop-typing-sherpa-test-{}",
            std::process::id()
        ));
        let model = root.join("models").join("builtin").join("m1");
        std::fs::create_dir_all(&model).unwrap();

        let result = find_in_dirs("m1", &[&root]);
        assert_eq!(result.as_deref(), Some(model.as_path()));
        assert_eq!(find_in_dirs("missing", &[&root]), None);

        let _ = std::fs::remove_dir_all(&root);
    }
}
