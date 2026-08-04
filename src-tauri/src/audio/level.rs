//! 试音电平表：独立 cpal 输入流，实时计算 RMS 并归一化为 0..1，
//! 以约 10Hz 频率推送给设置页音量条。
//!
//! 只在试音期间运行（`start-sound-test` → `stop-sound-test`），不常驻。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use cpal::traits::{DeviceTrait, StreamTrait};
use tauri::{AppHandle, Emitter};

use super::devices::resolve_input_device;
use crate::config::Config;

/// 静音阈值（RMS，与 pipeline 静音检测一致）
const SILENCE_RMS_THRESHOLD: f32 = 0.02;
/// RMS → 0..1 的放大系数：说话时典型 RMS（0.05~0.5）能顶满进度条
const LEVEL_GAIN: f32 = 5.0;
/// 电平推送间隔
const EMIT_INTERVAL: Duration = Duration::from_millis(100);

/// 启动试音电平表（阻塞直到 `stop` 被置位或流出错）。
///
/// 调用方负责单实例保护与 `stop` 复位。
pub fn run_sound_level_meter(app: AppHandle, stop: Arc<AtomicBool>) -> Result<()> {
    let (cfg, _) = Config::load_lenient();
    let device = resolve_input_device(cfg.audio.input_device.as_deref())?;
    let config = device
        .default_input_config()
        .map_err(|e| anyhow::anyhow!("读取麦克风配置失败：{e}"))?;
    let sample_format = config.sample_format();
    let channels = config.channels() as usize;
    let stream_config = config.config();

    let (rms_tx, rms_rx) = mpsc::channel::<f32>();
    let err_cb = |e| eprintln!("[drop-typing] 试音流错误：{e}");
    let stream = match sample_format {
        cpal::SampleFormat::F32 => {
            let tx = rms_tx.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[f32], _| {
                    let _ = tx.send(chunk_rms(data, channels));
                },
                err_cb,
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let tx = rms_tx.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[i16], _| {
                    let f: Vec<f32> =
                        data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                    let _ = tx.send(chunk_rms(&f, channels));
                },
                err_cb,
                None,
            )
        }
        cpal::SampleFormat::U16 => {
            let tx = rms_tx.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[u16], _| {
                    let f: Vec<f32> = data
                        .iter()
                        .map(|&s| (s as f32 - 32768.0) / 32768.0)
                        .collect();
                    let _ = tx.send(chunk_rms(&f, channels));
                },
                err_cb,
                None,
            )
        }
        other => {
            return Err(anyhow::anyhow!("不支持的采样格式：{other}"));
        }
    }
    .map_err(|e| anyhow::anyhow!("构建试音输入流失败：{e}"))?;
    stream
        .play()
        .map_err(|e| anyhow::anyhow!("启动试音输入流失败：{e}"))?;

    // 主循环：按固定窗口（100ms）收样本 → 归一化推送；stop 置位即退出。
    // 注意：不能用单次 recv_timeout 判断窗口结束——麦克风回调会持续送样本，
    // 永远等不到超时；这里按「真实时间窗口」收满再推送。
    loop {
        let window_start = Instant::now();
        let mut window_max: f32 = 0.0;
        while window_start.elapsed() < EMIT_INTERVAL {
            let remaining = EMIT_INTERVAL.saturating_sub(window_start.elapsed());
            match rms_rx.recv_timeout(remaining) {
                Ok(rms) => window_max = window_max.max(rms),
                Err(mpsc::RecvTimeoutError::Timeout) => break, // 窗口结束
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    drop(stream); // 回调已停止，直接退出
                    return Ok(());
                }
            }
        }
        let level = if window_max < SILENCE_RMS_THRESHOLD {
            0.0
        } else {
            (window_max * LEVEL_GAIN).min(1.0)
        };
        let _ = app.emit(
            "drop-typing://sound-level",
            serde_json::json!({ "level": level }),
        );
        if stop.load(Ordering::Relaxed) {
            break;
        }
    }
    drop(stream); // 停止输入流
    Ok(())
}

/// 计算一段样本（取第一声道）的 RMS。
fn chunk_rms(data: &[f32], channels: usize) -> f32 {
    let ch = channels.max(1);
    let n = (data.len() / ch).max(1);
    let mut sum_sq: f32 = 0.0;
    for frame in data.chunks(ch) {
        let s = frame[0];
        sum_sq += s * s;
    }
    (sum_sq / n as f32).sqrt()
}
