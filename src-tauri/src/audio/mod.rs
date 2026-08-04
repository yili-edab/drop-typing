//! 录音模块。
//!
//! cpal 本身是跨平台的（macOS CoreAudio / Windows WASAPI），
//! 这里不另做平台抽象，只把"录音 → 16kHz 单声道 WAV"封装为 `AudioRecorder`。
//!
//! `listener` 模块提供持续监听能力（唤醒词场景）。

pub mod devices;
pub mod level;
pub mod listener;
pub mod recorder;

pub use devices::{list_input_devices, resolve_input_device};
pub use level::run_sound_level_meter;
pub use listener::{ContinuousListener, RingBuffer, TailReader};
pub use recorder::AudioRecorder;
