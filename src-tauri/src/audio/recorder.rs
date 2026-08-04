//! 基于 cpal 的录音器。
//!
//! 两种用法：
//! - 批量：`start(None)` → `stop()` 取回 16kHz 单声道 16bit WAV 字节（M1 HTTP 方案）
//! - 流式：`start(Some(pcm_tx))` → 录音过程中持续把 PCM chunk
//!   （s16le / 16kHz / mono）发给 `pcm_tx`（实时 ASR 方案）
//!
//! 设备原生采样率（常见 44.1k/48k）经线性插值重采样到 16kHz，多声道取第一声道。
//! 流式重采样用带状态的 `StreamingResampler` 保持跨 chunk 连续性。
//!
//! 线程模型：专用音频线程持有 cpal Stream（Stream 在部分平台上不便于跨线程移动），
//! 通过命令通道接收 Start / Stop / Discard。

use std::io::Cursor;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, StreamTrait};

use super::devices::resolve_input_device;

const TARGET_RATE: u32 = 16_000;

enum Command {
    /// 可选的 PCM 流式输出通道（s16le 16k mono）
    Start(Option<mpsc::Sender<Vec<u8>>>),
    Stop(mpsc::Sender<Result<Vec<u8>, String>>),
    Discard,
}

pub struct AudioRecorder {
    tx: mpsc::Sender<Command>,
}

impl AudioRecorder {
    /// 创建录音器并启动音频线程（跟随系统默认输入设备）。设备缺失时立即报错。
    pub fn new() -> Result<Self> {
        Self::new_with_device(None)
    }

    /// 创建录音器并指定输入设备名（`None` = 跟随系统默认）。
    ///
    /// 配置的设备不存在时自动回退系统默认；两者都不可用时立即报错。
    pub fn new_with_device(device_name: Option<&str>) -> Result<Self> {
        let device_name = device_name.map(|s| s.to_string());
        resolve_input_device(device_name.as_deref())?; // 提前校验，无可用设备立即报错
        let (tx, rx) = mpsc::channel::<Command>();
        std::thread::Builder::new()
            .name("drop-typing-audio".into())
            .spawn(move || audio_thread(rx, device_name))?;
        Ok(Self { tx })
    }

    /// 开始录音。传入 `Some(sender)` 时边录边产出 PCM chunk（实时 ASR），
    /// 传 `None` 时只缓冲（批量 ASR，stop() 取 WAV）。
    pub fn start(&self, pcm_sink: Option<mpsc::Sender<Vec<u8>>>) -> Result<()> {
        self.tx.send(Command::Start(pcm_sink))?;
        Ok(())
    }

    /// 停止录音并取回 WAV 字节（批量路径）
    pub fn stop(&self) -> Result<Vec<u8>> {
        let (rtx, rrx) = mpsc::channel();
        self.tx.send(Command::Stop(rtx))?;
        rrx.recv()?.map_err(|e| anyhow!(e))
    }

    /// 停止并丢弃（短按 / 组合键作废 / 实时路径不需要 WAV）
    pub fn discard(&self) {
        let _ = self.tx.send(Command::Discard);
    }
}

/// 录音回调共享状态：f32 原始样本缓冲（批量用）+ 流式输出（实时用）
struct CaptureState {
    raw: Vec<f32>,
    sink: Option<mpsc::Sender<Vec<u8>>>,
    resampler: Option<StreamingResampler>,
}

fn audio_thread(rx: mpsc::Receiver<Command>, device_name: Option<String>) {
    let device = match resolve_input_device(device_name.as_deref()) {
        Ok(d) => d,
        Err(e) => {
            for cmd in rx {
                if let Command::Stop(reply) = cmd {
                    let _ = reply.send(Err(e.to_string()));
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
    let state: Arc<Mutex<CaptureState>> = Arc::new(Mutex::new(CaptureState {
        raw: Vec::new(),
        sink: None,
        resampler: None,
    }));
    let mut stream: Option<cpal::Stream> = None;

    for cmd in rx {
        match cmd {
            Command::Start(sink) => {
                {
                    let mut st = state.lock().unwrap();
                    st.raw.clear();
                    st.sink = sink;
                    st.resampler = st
                        .sink
                        .as_ref()
                        .map(|_| StreamingResampler::new(in_rate, TARGET_RATE));
                }
                if stream.is_none() {
                    stream = build_stream(&device, &config, sample_format, channels, state.clone());
                }
            }
            Command::Stop(reply) => {
                stream = None; // drop 即停止
                let mut st = state.lock().unwrap();
                st.sink = None;
                st.resampler = None;
                let data = std::mem::take(&mut st.raw);
                drop(st);
                let _ = reply.send(encode_wav(&data, in_rate, channels));
            }
            Command::Discard => {
                stream = None;
                let mut st = state.lock().unwrap();
                st.raw.clear();
                st.sink = None;
                st.resampler = None;
            }
        }
    }
}

fn build_stream(
    device: &cpal::Device,
    config: &cpal::SupportedStreamConfig,
    format: cpal::SampleFormat,
    channels: usize,
    state: Arc<Mutex<CaptureState>>,
) -> Option<cpal::Stream> {
    let err = |e| eprintln!("[drop-typing] audio stream error: {e}");
    let cfg = config.config();

    // 每个回调：取第一声道 → 缓冲原始样本；如有流式输出则重采样 → s16le → 发送
    let handle = move |state: &Arc<Mutex<CaptureState>>, frames: &[f32]| {
        let mut st = state.lock().unwrap();
        st.raw.extend_from_slice(frames);
        if st.sink.is_some() {
            let mono: Vec<f32> = frames
                .chunks(channels.max(1))
                .map(|frame| frame[0])
                .collect();
            let mut out_f32 = Vec::new();
            if let Some(rs) = &mut st.resampler {
                rs.push(&mono, &mut out_f32);
            }
            if !out_f32.is_empty() {
                let mut bytes = Vec::with_capacity(out_f32.len() * 2);
                for s in out_f32 {
                    let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    bytes.extend_from_slice(&v.to_le_bytes());
                }
                if let Some(sink) = &st.sink {
                    let _ = sink.send(bytes); // 接收端退出后失败属正常
                }
            }
        }
    };

    let stream = match format {
        cpal::SampleFormat::F32 => {
            let st = state.clone();
            device.build_input_stream(
                &cfg,
                move |data: &[f32], _| handle(&st, data),
                err,
                None,
            )
        }
        cpal::SampleFormat::I16 => {
            let st = state.clone();
            device.build_input_stream(
                &cfg,
                move |data: &[i16], _| {
                    let f: Vec<f32> = data.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                    handle(&st, &f);
                },
                err,
                None,
            )
        }
        cpal::SampleFormat::U16 => {
            let st = state.clone();
            device.build_input_stream(
                &cfg,
                move |data: &[u16], _| {
                    let f: Vec<f32> = data
                        .iter()
                        .map(|&s| (s as f32 - 32768.0) / 32768.0)
                        .collect();
                    handle(&st, &f);
                },
                err,
                None,
            )
        }
        other => {
            eprintln!("[drop-typing] unsupported sample format: {other}");
            return None;
        }
    };
    match stream {
        Ok(s) => {
            if let Err(e) = s.play() {
                eprintln!("[drop-typing] stream play failed: {e}");
                return None;
            }
            Some(s)
        }
        Err(e) => {
            eprintln!("[drop-typing] build input stream failed: {e}");
            None
        }
    }
}

/// 带状态的线性重采样器：跨 chunk 保持位置连续
struct StreamingResampler {
    ratio: f64,   // in_rate / out_rate
    next_out: u64, // 已输出的样本数
    base: f64,    // buf[0] 对应的绝对输入位置
    buf: Vec<f32>,
}

impl StreamingResampler {
    fn new(in_rate: u32, out_rate: u32) -> Self {
        Self {
            ratio: in_rate as f64 / out_rate as f64,
            next_out: 0,
            base: 0.0,
            buf: Vec::new(),
        }
    }

    fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.buf.extend_from_slice(input);
        loop {
            let p = self.next_out as f64 * self.ratio;
            let idx = p - self.base;
            if idx < 0.0 {
                self.next_out += 1;
                continue;
            }
            let i = idx as usize;
            if i + 1 >= self.buf.len() {
                break; // 等更多输入
            }
            let frac = (idx - i as f64) as f32;
            let a = self.buf[i];
            let b = self.buf[i + 1];
            out.push(a + (b - a) * frac);
            self.next_out += 1;
        }
        // 丢弃不再使用的前缀（保留最后 2 个样本保证插值连续）
        if self.buf.len() > 8192 {
            let p = self.next_out as f64 * self.ratio;
            let keep_from = ((p - self.base) as usize).saturating_sub(2);
            if keep_from > 0 {
                let n = keep_from.min(self.buf.len());
                self.buf.drain(..n);
                self.base += n as f64;
            }
        }
    }
}

/// 批量路径：取第一声道 → 线性重采样到 16kHz → 16bit WAV
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
