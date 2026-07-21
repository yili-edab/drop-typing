//! 编排层：热键事件 → 录音 → ASR → 暂存条 → 提交。
//!
//! 时序（PRD 3.4）：
//! - 按下右 ⌘ 即开始录音；松开时判定：
//!   - 时长 < 250ms（短按）→ 丢弃录音，提交暂存条
//!   - 时长 ≥ 250ms（长按）→ 停止录音，送 ASR，结果追加到暂存条
//! - 录音期间若有其它键按下（组合键用法，如 ⌘Space），作废本次录音。
//!
//! M2（LLM 清洗）插入点：`spawn_transcribe` 拿到 ASR 文本后、
//! `staging.append` 之前。M3/M4 新热键通道：扩展 hotkey 事件与这里的 match。

use std::sync::mpsc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Listener};

use crate::asr::{self, AsrProvider};
use crate::audio::AudioRecorder;
use crate::config::Config;
use crate::hotkey::{self, HotkeyEvent};
use crate::inject::{self, Injector};
use crate::staging::Staging;
use std::sync::Arc;

enum State {
    Idle,
    Recording { started: Instant, tainted: bool },
}

pub fn start(app: AppHandle) {
    let staging = Staging::new(app.clone());
    let (cfg, warning) = Config::load_lenient();
    let provider = asr::provider_from_config(&cfg);
    let injector = inject::default_injector();
    let source = hotkey::default_source();

    // 启动诊断：配置 / 权限问题直接以黄底红字显示在暂存条
    if let Some(w) = warning {
        staging.error(&w);
    } else if provider.is_none() {
        staging.error("未配置 DashScope API Key。请编辑 config.toml 或设置环境变量 DASHSCOPE_API_KEY。");
    }
    if !source.permission_trusted() {
        staging.error(
            "未授予辅助功能权限：系统设置 → 隐私与安全性 → 辅助功能，\
             勾选本应用（dev 模式下是运行它的终端）后重启应用。",
        );
    }

    // 前端加载完成后请求重发状态（启动早期的事件可能在前端监听注册前发出）
    let staging_for_ready = staging.clone();
    app.listen("byk://ready", move |_| staging_for_ready.republish());

    let (tx, rx) = mpsc::channel::<HotkeyEvent>();
    let staging_for_listener = staging.clone();
    std::thread::spawn(move || {
        if let Err(e) = source.start(tx) {
            staging_for_listener.error(&format!("全局热键监听启动失败：{e}"));
        }
    });

    std::thread::spawn(move || {
        run_loop(cfg, provider, injector, staging, rx);
    });
}

fn run_loop(
    cfg: Config,
    provider: Option<Arc<dyn AsrProvider>>,
    injector: Box<dyn Injector>,
    staging: Staging,
    rx: mpsc::Receiver<HotkeyEvent>,
) {
    let recorder = match AudioRecorder::new() {
        Ok(r) => Some(r),
        Err(e) => {
            staging.error(&format!("麦克风初始化失败：{e}"));
            None
        }
    };

    let mut state = State::Idle;
    let threshold = Duration::from_millis(cfg.long_press_threshold_ms);

    for ev in rx {
        match ev {
            HotkeyEvent::Error(msg) => staging.error(&msg),

            HotkeyEvent::OtherKeyDown => {
                if let State::Recording { tainted, .. } = &mut state {
                    *tainted = true;
                }
            }

            HotkeyEvent::TriggerDown => {
                if !matches!(state, State::Idle) {
                    continue;
                }
                match &recorder {
                    Some(r) => {
                        let _ = r.start();
                        staging.set_recording(true);
                        state = State::Recording {
                            started: Instant::now(),
                            tainted: false,
                        };
                    }
                    None => staging.error("录音器不可用（麦克风初始化失败）"),
                }
            }

            HotkeyEvent::TriggerUp => {
                let State::Recording { started, tainted } = state else {
                    continue;
                };
                state = State::Idle;
                staging.set_recording(false);

                let Some(r) = &recorder else { continue };
                let duration = started.elapsed();

                if tainted {
                    // 右 ⌘ 被用作组合键修饰键（如 ⌘Space），作废
                    r.discard();
                } else if duration < threshold {
                    // 短按：提交
                    r.discard();
                    commit(&staging, injector.as_ref());
                } else {
                    // 长按：停止录音 → ASR → 追加到暂存条
                    match r.stop() {
                        Ok(wav) => spawn_transcribe(&staging, provider.clone(), wav),
                        Err(e) => staging.error(&format!("录音失败：{e}")),
                    }
                }
            }
        }
    }
}

/// 短按提交：暂存条 → 剪贴板 → Cmd+V → 恢复剪贴板 → 清空暂存条
fn commit(staging: &Staging, injector: &dyn Injector) {
    let text = staging.take();
    if text.trim().is_empty() {
        return;
    }
    match injector.paste_text(&text) {
        Ok(()) => staging.committed(),
        Err(e) => {
            // 提交失败不丢内容：回滚到暂存条
            staging.set_text(&text);
            staging.error(&format!("提交失败（内容已保留在暂存条）：{e}"));
        }
    }
}

fn spawn_transcribe(
    staging: &Staging,
    provider: Option<Arc<dyn AsrProvider>>,
    wav: Vec<u8>,
) {
    let staging = staging.clone();
    staging.set_busy(true);
    tauri::async_runtime::spawn(async move {
        let result = match provider {
            Some(p) => {
                // 上下文偏置：暂存条现有文本随请求传给 ASR（M1 适配器暂未使用，见 TODO）
                let ctx = staging.text();
                let ctx = if ctx.trim().is_empty() { None } else { Some(ctx) };
                p.transcribe(&wav, ctx.as_deref()).await
            }
            None => Err(anyhow::anyhow!(
                "未配置 DashScope API Key，无法转写。请编辑 config.toml。"
            )),
        };
        staging.set_busy(false);
        match result {
            Ok(text) if !text.trim().is_empty() => staging.append(text.trim()),
            Ok(_) => staging.error("ASR 返回空文本"),
            Err(e) => staging.error(&format!("ASR 失败：{e:#}")),
        }
    });
}
