//! ASR 手动测试入口（不启动 Tauri 应用）。
//!
//! 用法：
//!   DASHSCOPE_API_KEY=sk-xxx cargo run --example test_asr -- path/to/audio.wav
//!
//! Key 也可以放在 config.toml（见 config.example.toml）。
//! WAV 为 16kHz 单声道最佳；其它格式会直接按原字节上传，由服务端判断。

use break_your_keyboard_lib::asr;
use break_your_keyboard_lib::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("用法: cargo run --example test_asr -- <path/to.wav>"))?;
    let wav = std::fs::read(&path)?;
    println!("[test_asr] 读取 {}（{} 字节）", path, wav.len());

    let (cfg, warning) = Config::load_lenient();
    if let Some(w) = warning {
        eprintln!("[test_asr] 配置警告：{w}");
    }
    let provider = asr::provider_from_config(&cfg).ok_or_else(|| {
        anyhow::anyhow!("未找到 API Key：请设置 DASHSCOPE_API_KEY 或编辑 config.toml")
    })?;

    println!("[test_asr] 调用 DashScope（model={}）...", cfg.asr_model);
    let text = provider.transcribe(&wav, None).await?;
    println!("[test_asr] 转写结果：{text}");
    Ok(())
}
