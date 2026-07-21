//! 基于 cpal 的录音器：输出 16kHz 单声道 16bit WAV 字节。
//!
//! 设备原生采样率（常见 44.1k/48k）经线性插值重采样到 16kHz，
//! 多声道取第一声道。M1 足够；后续可换 rubato 提升重采样质量。
//!
//! 线程模型：专用音频线程持有 cpal Stream（Stream 在部分平台上不便于跨线程移动），
//! 通过命令通道接收 Start / Stop / Discard。

use std::io::Cursor;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

const TARGET_RATE: u32 = 16_000;

enum Command {
    Start,
    Stop(mpsc::Sender<Result<Vec<u8>, String>>),
    Discard,
}

pub struct AudioRecorder {
    tx: mpsc::Sender<Command>,
}

impl AudioRecorder {
    /// 创建录音器并启动音频线程。设备缺失时立即报错。
    pub fn new() -> Result<Self> {
        // 提前探测输入设备，让错误在启动时暴露
        let host = cpal::default_host();
        if host.default_input_device().is_none() {
            return Err(anyhow!("未找到可用的麦克风设备"));
        }
        let (tx, rx) = mpsc::channel::<Command>();
        std::thread::Builder::new()
            .name("byk-audio".into())
            .spawn(move || audio_thread(rx))?;
        Ok(Self { tx })
    }

    /// 开始录音（重复调用会清空已有缓冲重新开始）
    pub fn start(&self) -> Result<()> {
        self.tx.send(Command::Start)?;
        Ok(())
    }

    /// 停止录音并取回 WAV 字节
    pub fn stop(&self) -> Result<Vec<u8>> {
        let (rtx, rrx) = mpsc::channel();
        self.tx.send(Command::Stop(rtx))?;
        rrx.recv()?.map_err(|e| anyhow!(e))
    }

    /// 停止并丢弃（短按 / 修饰键组合触发的录音作废）
    pub fn discard(&self) {
        let _ = self.tx.send(Command::Discard);
    }
}

fn audio_thread(rx: mpsc::Receiver<Command>) {
    let host = cpal::default_host();
    let device = match host.default_input_device() {
        Some(d) => d,
        None => {
            // 设备消失：把所有 Stop 都回错
            for cmd in rx {
                if let Command::Stop(reply) = cmd {
                    let _ = reply.send(Err("未找到可用的麦克风设备".into()));
                }
            }
            return;
        }
    };
    let config = match device.default_input_config() {
        Ok(c) => c,
        Err(e) => {
            for cmd in rx {
                if let Command::Stop(reply) = cmd {
                    let _ = reply.send(Err(format!("读取麦克风配置失败：{e}")));
                }
            }
            return;
        }
    };

    let in_rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let sample_format = config.sample_format();
    let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
    let mut stream: Option<cpal::Stream> = None;

    for cmd in rx {
        match cmd {
            Command::Start => {
                samples.lock().unwrap().clear();
                if stream.is_none() {
                    stream = build_stream(&device, &config, sample_format, samples.clone());
                }
            }
            Command::Stop(reply) => {
                stream = None; // drop 即停止
                let data = std::mem::take(&mut *samples.lock().unwrap());
                let _ = reply.send(encode_wav(&data, in_rate, channels));
            }
            Command::Discard => {
                stream = None;
                samples.lock().unwrap().clear();
            }
        }
    }
}

fn build_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    format: cpal::SampleFormat,
    samples: Arc<Mutex<Vec<f32>>>,
) -> Option<cpal::Stream> {
    let err = |e| eprintln!("[byk] audio stream error: {e}");
    let cfg = config.config();
    let stream = match format {
        cpal::SampleFormat::F32 => {
            let buf = samples.clone();
            device.build_input_stream(
                &cfg,
                move |data: &[f32], _| buf.lock().unwrap().extend_from_slice(data),
                err,
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let buf = samples.clone();
            device.build_input_stream(
                &cfg,
                move |data: &[i16], _| {
                    let mut b = buf.lock().unwrap();
                    b.extend(data.iter().map(|&s| s as f32 / i16::MAX as f32));
                },
                err,
                None,
            )
        }
        cpal::SampleFormat::U16 => {
            let buf = samples.clone();
            device.build_input_stream(
                &cfg,
                move |data: &[u16], _| {
                    let mut b = buf.lock().unwrap();
                    b.extend(
                        data.iter()
                            .map(|&s| (s as f32 - 32768.0) / 32768.0),
                    );
                },
                err,
                None,
            )
        }
        other => {
            eprintln!("[byk] unsupported sample format: {other}");
            return None;
        }
    };
    match stream {
        Ok(s) => {
            if let Err(e) = s.play() {
                eprintln!("[byk] stream play failed: {e}");
                return None;
            }
            Some(s)
        }
        Err(e) => {
            eprintln!("[byk] build input stream failed: {e}");
            None
        }
    }
}

/// 取第一声道 → 线性重采样到 16kHz → 16bit WAV
fn encode_wav(samples: &[f32], in_rate: u32, channels: usize) -> Result<Vec<u8>, String> {
    if samples.is_empty() {
        return Err("没有录到音频（麦克风权限未授予？）".into());
    }
    let channels = channels.max(1);
    let mono: Vec<f32> = samples.chunks(channels).map(|frame| frame[0]).collect();
    let resampled = resample_linear(&mono, in_rate, TARGET_RATE);

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: TARGET_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec)
            .map_err(|e| format!("创建 WAV 写入器失败：{e}"))?;
        for &s in &resampled {
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            writer
                .write_sample(v)
                .map_err(|e| format!("写入采样失败：{e}"))?;
        }
        writer.finalize().map_err(|e| format!("WAV 收尾失败：{e}"))?;
    }
    Ok(cursor.into_inner())
}

fn resample_linear(input: &[f32], in_rate: u32, out_rate: u32) -> Vec<f32> {
    if input.is_empty() || in_rate == out_rate {
        return input.to_vec();
    }
    let ratio = in_rate as f64 / out_rate as f64;
    let out_len = (input.len() as f64 / ratio) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos as usize;
        let frac = (pos - idx as f64) as f32;
        let a = input[idx];
        let b = input.get(idx + 1).copied().unwrap_or(a);
        out.push(a + (b - a) * frac);
    }
    out
}
