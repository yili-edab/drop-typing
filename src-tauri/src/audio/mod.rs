//! 录音模块。
//!
//! cpal 本身是跨平台的（macOS CoreAudio / Windows WASAPI），
//! 这里不另做平台抽象，只把"录音 → 16kHz 单声道 WAV"封装为 `AudioRecorder`。

pub mod recorder;

pub use recorder::AudioRecorder;
