//! ASR 手动测试入口（不启动 Tauri 应用）。
//!
//! 用法：
//!   cargo run --example test_asr -- path/to/audio.wav
//!
//! - 配置读取顺序与 App 一致：~/.break-your-keyboard.toml →
//!   ~/Library/Application Support/break-your-keyboard/config.toml → 环境变量 DASHSCOPE_API_KEY
//! - provider = bailian-realtime（默认）：要求 WAV 为 16kHz 单声道 16bit，
//!   模拟实时流（每 100ms 送 3200 字节），打印中间结果与最终全文
//! - provider = bailian：整段 WAV 一次性上传（原始字节直传）

use std::io::Write as _;
use std::sync::mpsc;
use std::time::Duration;

use break_your_keyboard_lib::asr::{self, AsrBackend};
use break_your_keyboard_lib::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("用法: cargo run --example test_asr -- <path/to.wav>"))?;
    let bytes = std::fs::read(&path)?;
    println!("[test_asr] 读取 {}（{} 字节）", path, bytes.len());

    let (cfg, warning) = Config::load_lenient();
    if let Some(w) = warning {
        eprintln!("[test_asr] 配置警告：{w}");
    }
    println!(
        "[test_asr] provider={} model={}",
        cfg.asr.provider,
        cfg.asr_model_name()
    );
    let backend = asr::backend_from_config(&cfg)
        .ok_or_else(|| anyhow::anyhow!("未找到 API Key 或 provider 未知"))?;

    match backend {
        AsrBackend::Realtime(p) => {
            // 解析 WAV：要求 16kHz 单声道 16bit
            let reader = hound::WavReader::new(std::io::Cursor::new(bytes))?;
            let spec = reader.spec();
            if spec.sample_rate != 16000 || spec.channels != 1 || spec.bits_per_sample != 16 {
                anyhow::bail!(
                    "realtime 测试要求 16kHz 单声道 16bit WAV，当前：{}Hz {}ch {}bit。\n\
                     可用 say 生成：say -o test.wav --data-format=LEI16@16000 \"你好，世界\"",
                    spec.sample_rate,
                    spec.channels,
                    spec.bits_per_sample
                );
            }
            let mut pcm: Vec<u8> = Vec::new();
            for s in reader.into_samples::<i16>() {
                pcm.extend_from_slice(&s?.to_le_bytes());
            }
            println!("[test_asr] PCM {} 字节，建立实时会话...", pcm.len());

            let (ptx, prx) = mpsc::channel::<String>();
            std::thread::spawn(move || {
                for text in prx {
                    print!("\r[test_asr] 中间结果: {text}          ");
                    let _ = std::io::stdout().flush();
                }
            });

            let session = p.start_session(ptx)?;
            println!("[test_asr] 会话建立（task-started），开始送音频...");

            // 100ms @16k16bitmono = 3200 字节
            for chunk in pcm.chunks(3200) {
                session.send_audio(chunk)?;
                std::thread::sleep(Duration::from_millis(100));
            }
            let text = session.finish()?;
            println!("\n[test_asr] 最终转写：{text}");
        }
        AsrBackend::Batch(p) => {
            println!("[test_asr] 调用 DashScope HTTP（整段上传）...");
            let text = p.transcribe(&bytes, None).await?;
            println!("[test_asr] 转写结果：{text}");
        }
    }
    Ok(())
}
