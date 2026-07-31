//! 持续音频监听器 + 环形缓冲区。
//!
//! 与 `recorder.rs` 不同，本模块维护一条始终运行的 cpal 音频流，
//! 数据写入 `RingBuffer` 供唤醒词引擎和 ASR 录制共用。
//!
//! 线程安全：RingBuffer 为单写者多读者设计，写者只追加、读者只读取已写入区域。

use std::cell::UnsafeCell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::wakeword::sherpa::SherpaKws;
use crate::wakeword::{WakeEvent, WakeWord};

const TARGET_RATE: u32 = 16_000;

// ── RingBuffer ───────────────────────────────────────────────────────

/// 无锁环形缓冲区（单写者多读者）。
///
/// 使用单调递增的 `write_pos`（绝对采样序号）代替传统 head/tail 指针。
/// 写入后更新 `write_pos`；读者按绝对位置 [start, end) 读取，
/// 只要 start ≥ write_pos - capacity 即可检索到数据。
///
/// 内部用 `UnsafeCell` 实现写入时的内部可变性（单写者保证安全）。
pub struct RingBuffer {
    buf: UnsafeCell<Vec<f32>>,
    capacity: u64,
    write_pos: AtomicU64,
}

// 安全：仅单写者访问 UnsafeCell，读者仅读取已发布的数据
unsafe impl Sync for RingBuffer {}
unsafe impl Send for RingBuffer {}

impl RingBuffer {
    /// `duration_ms` 毫秒 @ 16kHz 的容量。
    pub fn new(duration_ms: u64) -> Self {
        let capacity = (duration_ms as u64 * TARGET_RATE as u64) / 1000;
        Self {
            buf: UnsafeCell::new(vec![0.0_f32; capacity as usize]),
            capacity,
            write_pos: AtomicU64::new(0),
        }
    }

    /// 音频回调中调用：写入采样。返回写入后的绝对位置。
    pub fn write(&self, samples: &[f32]) -> u64 {
        let buf = unsafe { &mut *self.buf.get() };
        let mut pos = self.write_pos.load(Ordering::Relaxed);
        for &s in samples {
            buf[(pos % self.capacity) as usize] = s;
            pos += 1;
        }
        self.write_pos.store(pos, Ordering::Release);
        pos
    }

    /// 当前绝对写入位置。
    pub fn position(&self) -> u64 {
        self.write_pos.load(Ordering::Acquire)
    }

    /// 按绝对位置读取采样 [start, end)。
    ///
    /// 若 `end > position()` 或数据已因超出容量被覆盖，返回 `None`。
    pub fn read(&self, start: u64, end: u64) -> Option<Vec<f32>> {
        let current = self.position();
        if end > current || start > end {
            return None;
        }
        let oldest = current.saturating_sub(self.capacity);
        if start < oldest {
            return None; // 已被覆盖
        }
        let len = (end - start) as usize;
        let mut out = Vec::with_capacity(len);
        let buf = unsafe { &*self.buf.get() };
        for i in start..end {
            out.push(buf[(i % self.capacity) as usize]);
        }
        Some(out)
    }
}

// ── TailReader ────────────────────────────────────────────────────────

/// 环形缓冲区尾随读取器。
///
/// 从一个给定的绝对位置开始，持续跟随 `write_pos` 读取最新数据。
/// 用于唤醒词触发后把 RingBuffer 中的音频 chunk 喂给 ASR。
pub struct TailReader {
    buffer: Arc<RingBuffer>,
    next_read: u64,
}

impl TailReader {
    pub fn new(buffer: Arc<RingBuffer>, from: u64) -> Self {
        Self {
            buffer,
            next_read: from,
        }
    }

    /// 读取从上次位置到当前 write_pos 之间所有可用采样。
    /// 无新数据时返回空 Vec。
    pub fn read_available(&mut self) -> Vec<f32> {
        let current = self.buffer.position();
        if self.next_read >= current {
            return Vec::new();
        }
        match self.buffer.read(self.next_read, current) {
            Some(samples) => {
                self.next_read = current;
                samples
            }
            None => {
                // 数据已被覆盖（极其罕见：读取速度落后超过 capacity）
                // 跳过丢失部分，从最早可用位置继续
                let oldest = current.saturating_sub(self.buffer.capacity);
                self.next_read = oldest;
                self.buffer.read(oldest, current).unwrap_or_default()
            }
        }
    }
}

// ── Resampler ──────────────────────────────────────────────────────────

/// 简单线性插值重采样器：输入任意采样率 → 输出固定 16kHz。
struct Resampler {
    input_rate: u32,
    output_rate: u32,
    /// 已累积的输入采样（不包含被消费的部分）
    buffer: Vec<f32>,
}

impl Resampler {
    fn new(input_rate: u32) -> Self {
        Self {
            input_rate,
            output_rate: TARGET_RATE,
            buffer: Vec::new(),
        }
    }

    /// 喂入 `input` 采样，返回重采样后的 16kHz 输出。
    ///
    /// 内部保留不足一个输出采样的余量，供下一次调用补齐插值。
    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        self.buffer.extend_from_slice(input);

        let ratio = self.input_rate as f64 / self.output_rate as f64; // > 1 为降采样
        if ratio <= 0.0 || self.buffer.len() < 2 {
            return Vec::new();
        }

        // 可产出的最大输出索引
        let max_out = ((self.buffer.len() - 1) as f64 / ratio).floor() as usize;
        if max_out == 0 {
            return Vec::new();
        }

        let mut output = Vec::with_capacity(max_out);
        for j in 0..max_out {
            let pos = j as f64 * ratio;
            let i = pos as usize;
            let frac = pos - i as f64;
            let s = self.buffer[i] as f64
                + (self.buffer[i + 1] as f64 - self.buffer[i] as f64) * frac;
            output.push(s as f32);
        }

        // 保留未消费的余量：最后一个输出位置之后的输入样本
        // consumed ≈ ceil(max_out * ratio)
        let consumed = ((max_out - 1) as f64 * ratio).ceil() as usize + 1;
        let keep = consumed.min(self.buffer.len());
        if keep > 0 {
            self.buffer.drain(0..keep);
        }

        output
    }
}

// ── ContinuousListener ────────────────────────────────────────────────

/// 持续音频监听器。
///
/// 持有始终运行的 cpal 输入流，数据写入共享的 `RingBuffer`。
pub struct ContinuousListener {
    pub buffer: Arc<RingBuffer>,
    /// 发送给 cpal Stream 的停止信号（drop 时停止流）
    _stream: cpal::Stream,
}

impl ContinuousListener {
    /// 创建监听器并启动 cpal 流。
    ///
    /// `duration_ms` 决定环形缓冲区容量。设备不可用时返回错误。
    pub fn new(duration_ms: u64) -> anyhow::Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .ok_or_else(|| anyhow::anyhow!("未找到可用的麦克风设备"))?;
        let config = device
            .default_input_config()
            .map_err(|e| anyhow::anyhow!("读取麦克风配置失败：{e}"))?;
        let sample_format = config.sample_format();
        let channels = config.channels() as usize;
        let in_rate = config.sample_rate().0;

        eprintln!(
            "[drop-typing] 持续监听：{:.1}kHz {}ch {:?} → 重采样至 16kHz",
            in_rate as f64 / 1000.0,
            channels,
            sample_format,
        );

        let buffer = Arc::new(RingBuffer::new(duration_ms));
        let buf = buffer.clone();

        // 使用设备默认配置（不做强制采样率请求，避免不兼容）
        let cfg = config.config();

        // 需要重采样吗？
        let need_resample = in_rate != TARGET_RATE;
        let resampler: Option<Arc<std::sync::Mutex<Resampler>>> =
            if need_resample { Some(Arc::new(std::sync::Mutex::new(Resampler::new(in_rate)))) } else { None };

        let stream: cpal::Stream = match sample_format {
            cpal::SampleFormat::F32 => {
                let buf = buf.clone();
                let rs = resampler.clone();
                device.build_input_stream(
                    &cfg,
                    move |data: &[f32], _| {
                        let mono: Vec<f32> = data
                            .chunks(channels.max(1))
                            .map(|frame| frame[0])
                            .collect();
                        if let Some(ref rs) = rs {
                            let out = rs.lock().unwrap().process(&mono);
                            if !out.is_empty() {
                                buf.write(&out);
                            }
                        } else {
                            buf.write(&mono);
                        }
                    },
                    |e| eprintln!("[drop-typing] listener stream error: {e}"),
                    None,
                )
            }
            cpal::SampleFormat::I16 => {
                let buf = buf.clone();
                let rs = resampler.clone();
                device.build_input_stream(
                    &cfg,
                    move |data: &[i16], _| {
                        let mono: Vec<f32> = data
                            .chunks(channels.max(1))
                            .map(|frame| frame[0] as f32 / i16::MAX as f32)
                            .collect();
                        if let Some(ref rs) = rs {
                            let out = rs.lock().unwrap().process(&mono);
                            if !out.is_empty() {
                                buf.write(&out);
                            }
                        } else {
                            buf.write(&mono);
                        }
                    },
                    |e| eprintln!("[drop-typing] listener stream error: {e}"),
                    None,
                )
            }
            cpal::SampleFormat::U16 => {
                let buf = buf.clone();
                let rs = resampler.clone();
                device.build_input_stream(
                    &cfg,
                    move |data: &[u16], _| {
                        let mono: Vec<f32> = data
                            .chunks(channels.max(1))
                            .map(|frame| (frame[0] as f32 - 32768.0) / 32768.0)
                            .collect();
                        if let Some(ref rs) = rs {
                            let out = rs.lock().unwrap().process(&mono);
                            if !out.is_empty() {
                                buf.write(&out);
                            }
                        } else {
                            buf.write(&mono);
                        }
                    },
                    |e| eprintln!("[drop-typing] listener stream error: {e}"),
                    None,
                )
            }
            other => {
                return Err(anyhow::anyhow!(
                    "不支持的采样格式：{other}"
                ));
            }
        }
        .map_err(|e| anyhow::anyhow!("构建音频输入流失败：{e}"))?;

        stream
            .play()
            .map_err(|e| anyhow::anyhow!("启动音频流失败：{e}"))?;

        Ok(Self {
            buffer: buf,
            _stream: stream,
        })
    }

    /// 启动唤醒词检测线程。
    ///
    /// 线程内每 80ms（1280 采样 @ 16kHz）从 RingBuffer 读一帧，
    /// 通过 sherpa-onnx 流式 KeywordSpotter 推理，
    /// 检测到关键词后发 `WakeEvent`。
    ///
    /// 返回 `wake_rx` 接收端，供 pipeline 轮询。
    pub fn start_wake_word(
        buffer: Arc<RingBuffer>,
        kws: SherpaKws,
    ) -> mpsc::Receiver<WakeEvent> {
        let (tx, rx) = mpsc::channel::<WakeEvent>();
        let frame_samples = (80 * TARGET_RATE as u64 / 1000) as usize; // 1280

        std::thread::Builder::new()
            .name("drop-typing-wakeword".into())
            .spawn(move || {
                let mut stream = kws.create_stream();
                let mut next_frame_start: u64 = 0;

                // 去抖：首次检测后启动固定窗口，窗口内持续收集更长的关键词，
                // 到期后发送窗口内最长的那个。不重置计时器以保证延迟一致。
                let mut pending_word: Option<WakeWord> = None;
                let mut pending_position: u64 = 0;
                let mut pending_since: Option<std::time::Instant> = None;
                const DEBOUNCE_MS: u64 = 800; // 固定去抖窗口

                loop {
                    // 去抖窗口到期 → 发送收集到的最长关键词
                    if let (Some(ref pw), Some(since)) = (&pending_word, pending_since) {
                        if since.elapsed().as_millis() as u64 >= DEBOUNCE_MS {
                            let _ = tx.send(WakeEvent::Detected {
                                word: pw.clone(),
                                position: pending_position,
                            });
                            eprintln!(
                                "[drop-typing] 唤醒词去抖完成，发送：'{}' action='{}'",
                                pw.text, pw.action,
                            );
                            next_frame_start = pending_position + TARGET_RATE as u64;
                            pending_word = None;
                            pending_since = None;
                            kws.reset(&mut stream);
                            continue;
                        }
                    }
                    // 等待足够数据
                    let current = buffer.position();
                    let needed = next_frame_start + frame_samples as u64;
                    if current < needed {
                        std::thread::sleep(std::time::Duration::from_millis(10));
                        continue;
                    }

                    let frame = match buffer.read(next_frame_start, needed) {
                        Some(f) => f,
                        None => {
                            // 数据已被覆盖，跳到最新
                            next_frame_start = buffer
                                .position()
                                .saturating_sub(frame_samples as u64);
                            continue;
                        }
                    };

                    // sherpa-onnx 流式推理
                    if let Some(word) = kws.process_frame(&mut stream, &frame) {
                        match &pending_word {
                            Some(prev) if word.text.len() > prev.text.len() => {
                                eprintln!(
                                    "[drop-typing] 唤醒词去抖更新：'{}' → '{}'（{:.0}ms）",
                                    prev.text, word.text,
                                    pending_since.unwrap().elapsed().as_millis(),
                                );
                                pending_word = Some(word.clone());
                                pending_position = needed;
                                // 不重置 pending_since，保证总延迟固定
                                kws.reset(&mut stream);
                            }
                            Some(prev) => {
                                eprintln!(
                                    "[drop-typing] 唤醒词去抖忽略：'{}'（不长于'{}'）",
                                    word.text, prev.text,
                                );
                                kws.reset(&mut stream);
                            }
                            None => {
                                eprintln!(
                                    "[drop-typing] 唤醒词去抖启动：'{}' action='{}'",
                                    word.text, word.action,
                                );
                                pending_word = Some(word.clone());
                                pending_position = needed;
                                pending_since = Some(std::time::Instant::now());
                                kws.reset(&mut stream);
                            }
                        }
                        next_frame_start = needed;
                        continue;
                    }

                    next_frame_start = needed;

                    // 检查是否需要跳过（追赶上实时）
                    let lag = buffer.position().saturating_sub(next_frame_start);
                    if lag > frame_samples as u64 * 2 {
                        next_frame_start =
                            buffer.position().saturating_sub(frame_samples as u64);
                    }
                }
            })
            .expect("启动唤醒词线程失败");

        rx
    }
}
