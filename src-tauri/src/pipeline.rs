//! 编排层：热键事件 → 录音 → ASR → 暂存条 → 提交。
//!
//! 时序（PRD 3.4）：
//! - 按下右 ⌘ 即开始录音；松开时判定：
//!   - 时长 < 150ms（短按）→ 丢弃录音，提交暂存条
//!   - 时长 ≥ 150ms（长按）→ 停止录音，送 ASR，结果追加到暂存条
//! - 录音期间若有其它键按下（组合键用法，如 ⌘Space），作废本次录音。
//!
//! 两种 ASR 后端：
//! - 实时（Realtime）：按下时建立 WebSocket 会话，边录边传 PCM，
//!   中间结果实时推到暂存条；松开时 finish 取最终全文。
//! - 批量（Batch）：录完整个 WAV 一次性 HTTP 转写（M1 方案，备选）。
//!
//! M2：拿到最终文本后、`staging.append` 之前，经 `clean_and_append` 过一道
//! LLM 清洗（未配置 `[llm]` 时直出，清洗失败降级为原文追加）。

use std::sync::mpsc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow;
use tauri::{AppHandle, Listener, Manager};

use crate::asr::{self, AsrBackend, RealtimeSession};
use crate::audio::{AudioRecorder, ContinuousListener, RingBuffer, TailReader};
use crate::command;
use crate::config::{Config, WakewordConfig};
use crate::hotkey::{self, HotkeyEvent};
use crate::inject::{self, Injector};
use crate::llm::{self, TextCleaner};
use crate::prompts;
use crate::staging::Staging;
use crate::wakeword::{self, WakeEvent, WakeWord};

/// 录音目的：输入通道（右 ⌘）、修正通道（右 ⌥）还是指令通道（右 ⇧，M4）。
#[derive(Clone, Copy, PartialEq, Eq)]
#[derive(Debug)]
pub(crate) enum RecordMode {
    Input,
    Repair,
    Command,
}

impl RecordMode {
    /// 识别期间状态徽章文案
    fn recognizing_label(self) -> &'static str {
        match self {
            RecordMode::Input | RecordMode::Repair => "识别中",
            RecordMode::Command => "指令识别中",
        }
    }
}

enum State {
    Idle,
    /// 持续监听中（唤醒词启用时）。等待 HotkeyEvent 或 WakeEvent。
    Listening,
    Recording {
        started: Instant,
        tainted: bool,
        mode: RecordMode,
        /// 暂存条是否已显示（按下后等阈值到期才 show，避免短按一闪而过）
        bar_shown: bool,
        /// 按下时暂存条正处于异常态：本次短按仅消除错误，不提交
        dismiss_only: bool,
        /// 若本次录音是从 PendingCommit 触发的，携带第一击时间用于双击判定
        pending_since: Option<Instant>,
        /// 实时后端的活动会话（批量后端为 None）
        session: Option<Arc<dyn RealtimeSession>>,
        /// 后台建连中：会话尚未就绪时暂存 Receiver，松手时取回
        pending_rx: Option<mpsc::Receiver<anyhow::Result<Arc<dyn RealtimeSession>>>>,
        /// 录音是否由鼠标侧键启动（用于跨设备 taint 判定）
        started_by_mouse: bool,
        /// 唤醒词触发（None = 热键触发）
        wake_word: Option<WakeWord>,
        /// 唤醒词录音的 ASR 结果接收端
        wake_finish_rx: Option<mpsc::Receiver<anyhow::Result<String>>>,
    },
    /// 输入通道短按后等待判定：超时则单击提交，窗口内再次短按则双击清空
    PendingCommit { since: Instant },
}

/// 运行时可热加载状态：模型后端、清洗器、指令词表与毫秒参数。
///
/// 设置页保存后通过 `drop-typing://runtime-reload` 事件重建；
/// 热键绑定与唤醒词引擎不在此列（启动时加载，改动需重启）。
struct RuntimeState {
    backend: Option<Arc<AsrBackend>>,
    cleaner: Option<Arc<dyn TextCleaner>>,
    lexicon: Arc<command::Lexicon>,
    threshold: Duration,
    double_press: Duration,
    command_countdown: Duration,
}

impl RuntimeState {
    fn from_config(cfg: &Config) -> Self {
        Self {
            backend: asr::backend_from_config(cfg).map(Arc::new),
            cleaner: llm::cleaner_from_config(cfg),
            lexicon: Arc::new(command::Lexicon::build(Some(&cfg.command))),
            threshold: Duration::from_millis(cfg.long_press_threshold_ms),
            double_press: Duration::from_millis(cfg.double_press_window_ms),
            command_countdown: Duration::from_millis(cfg.effective_command_countdown_ms()),
        }
    }
}

/// 唤醒词热开关命令（设置页保存后经 runtime-reload 触发）。
enum WakeCommand {
    Enable(WakewordConfig, Option<String>),
    Disable,
}

/// 录音器设备热切换命令（设置页保存 `[audio]` 后触发）。
enum AudioCommand {
    /// 按设备名重建录音器（None = 跟随系统默认）。
    ReloadDevice(Option<String>),
}

/// 唤醒词管理线程：独占持有 cpal 持续监听流（Stream 不能跨线程），
/// 收到命令后创建/停止监听，并把资源（环形缓冲 + 事件接收端）发给 pipeline。
fn wake_manager_loop(
    rx: mpsc::Receiver<WakeCommand>,
    out_tx: mpsc::Sender<Option<(WakewordConfig, Arc<RingBuffer>, mpsc::Receiver<WakeEvent>)>>,
    resource_dir: std::path::PathBuf,
) {
    let mut listener: Option<ContinuousListener> = None;
    while let Ok(cmd) = rx.recv() {
        match cmd {
            WakeCommand::Disable => {
                // drop 即停止 cpal 流，麦克风指示灯熄灭
                listener = None;
                let _ = out_tx.send(None);
            }
            WakeCommand::Enable(wcfg, device_name) => {
                // 配置变化时先停旧流，再按新配置重建
                listener = None;
                match ContinuousListener::new_with_device(
                    wcfg.ring_buffer_duration_ms,
                    device_name.as_deref(),
                ) {
                    Ok(l) => {
                        let engine = wakeword::create_engine(&wcfg, &resource_dir);
                        if let Some(eng) = engine {
                            let buf = l.buffer.clone();
                            let wake_rx = ContinuousListener::start_wake_word(buf.clone(), eng);
                            listener = Some(l);
                            let _ = out_tx.send(Some((wcfg, buf, wake_rx)));
                        } else {
                            eprintln!("[drop-typing] 唤醒词引擎创建失败（模型缺失？）");
                            let _ = out_tx.send(None);
                        }
                    }
                    Err(e) => {
                        eprintln!("[drop-typing] 唤醒词监听器启动失败：{e}");
                        let _ = out_tx.send(None);
                    }
                }
            }
        }
    }
}

pub fn start(app: AppHandle) {
    let staging = Staging::new(app.clone());
    let (cfg, warning) = Config::load_lenient();
    let injector = inject::default_injector(app.clone());
    let source = hotkey::default_source();
    let runtime = Arc::new(Mutex::new(RuntimeState::from_config(&cfg)));

    // 启动诊断：配置 / 权限问题直接以黄底红字显示在暂存条
    if let Some(w) = warning {
        staging.error(&w);
    } else if runtime.lock().unwrap().backend.is_none() {
        staging.error("未配置 ASR API Key 或 provider 未知。请检查配置文件（见 config.example.toml）。");
    }
    if !source.permission_trusted() {
        staging.error(
            "未授予辅助功能权限：系统设置 → 隐私与安全性 → 辅助功能，\
             勾选本应用（dev 模式下是运行它的终端）后重启应用。",
        );
    }

    // 前端加载完成后请求重发状态（启动早期的事件可能在前端监听注册前发出）
    let staging_for_ready = staging.clone();
    app.listen("drop-typing://ready", move |_| staging_for_ready.republish());

    // 润色样式选择（暂存条下拉框 → 更新当前样式）
    let current_style: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(
        cfg.llm.current_style.clone(),
    ));
    let style_for_listener = current_style.clone();
    let app_for_style = app.clone();
    app.listen("drop-typing://select-style", move |ev| {
        let payload: serde_json::Value =
            serde_json::from_str(ev.payload()).unwrap_or_default();
        let style = payload
            .get("style")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        *style_for_listener.lock().unwrap() = style;
        // 持久化当前样式选择到配置文件
        let (mut cfg, _) = Config::load_lenient();
        cfg.llm.current_style = style_for_listener.lock().unwrap().clone();
        let _ = cfg.save();
        // 重新发送样式信息以更新暂存条下拉框选中项
        crate::settings::emit_styles(&app_for_style);
    });

    // 唤醒词（Phase 1）：独立管理线程持有 cpal 持续监听流，
    // 支持运行中热开关（设置页保存后经 runtime-reload 命令重建/停止）。
    let (wake_cmd_tx, wake_cmd_rx) = mpsc::channel::<WakeCommand>();
    let (wake_out_tx, wake_out_rx) = mpsc::channel::<
        Option<(WakewordConfig, Arc<RingBuffer>, mpsc::Receiver<WakeEvent>)>,
    >();
    let wake_resource_dir = app.path().resource_dir().unwrap_or_default();
    std::thread::Builder::new()
        .name("drop-typing-wake-manager".into())
        .spawn(move || wake_manager_loop(wake_cmd_rx, wake_out_tx, wake_resource_dir))
        .expect("启动唤醒词管理线程失败");
    if cfg.wakeword.enabled {
        let _ = wake_cmd_tx.send(WakeCommand::Enable(
            cfg.wakeword.clone(),
            cfg.audio.input_device.clone(),
        ));
    }

    // 录音器设备热切换：设置页保存 `[audio]` 后由 runtime-reload 触发重建。
    let (audio_cmd_tx, audio_cmd_rx) = mpsc::channel::<AudioCommand>();

    // 设置页保存可热加载配置（模型 / 毫秒 / 指令词表 / 唤醒词）后，重建运行时状态。
    // 热键绑定保持启动时加载，改动需重启生效（由设置页提示）。
    let runtime_for_reload = runtime.clone();
    let staging_for_reload = staging.clone();
    let wake_cmd_for_reload = wake_cmd_tx.clone();
    let audio_cmd_for_reload = audio_cmd_tx.clone();
    let last_wakeword: Arc<Mutex<Option<WakewordConfig>>> = Arc::new(Mutex::new(None));
    *last_wakeword.lock().unwrap() = Some(cfg.wakeword.clone());
    let last_wakeword_for_reload = last_wakeword.clone();
    let last_audio: Arc<Mutex<Option<crate::config::AudioConfig>>> = Arc::new(Mutex::new(None));
    *last_audio.lock().unwrap() = Some(cfg.audio.clone());
    let last_audio_for_reload = last_audio.clone();
    app.listen("drop-typing://runtime-reload", move |_| {
        let (new_cfg, _) = Config::load_lenient();
        let mut g = runtime_for_reload.lock().unwrap();
        *g = RuntimeState::from_config(&new_cfg);
        if g.backend.is_none() {
            staging_for_reload.error(
                "未配置 ASR API Key 或 provider 未知。请检查配置文件（见 config.example.toml）。",
            );
        }
        // 音频设备段变化 → 重建录音器（空闲时立即换，录音中由 run_loop 延后）
        let audio_changed = last_audio_for_reload
            .lock()
            .unwrap()
            .as_ref()
            .map_or(true, |old| old != &new_cfg.audio);
        *last_audio_for_reload.lock().unwrap() = Some(new_cfg.audio.clone());
        if audio_changed {
            let _ = audio_cmd_for_reload.send(AudioCommand::ReloadDevice(
                new_cfg.audio.input_device.clone(),
            ));
        }
        // 唤醒词段或音频设备变化 → 热切换麦克风监听（无需重启）
        let wakeword_changed = last_wakeword_for_reload
            .lock()
            .unwrap()
            .as_ref()
            .map_or(true, |old| old != &new_cfg.wakeword);
        *last_wakeword_for_reload.lock().unwrap() = Some(new_cfg.wakeword.clone());
        if wakeword_changed || audio_changed {
            let _ = wake_cmd_for_reload.send(if new_cfg.wakeword.enabled {
                WakeCommand::Enable(
                    new_cfg.wakeword.clone(),
                    new_cfg.audio.input_device.clone(),
                )
            } else {
                WakeCommand::Disable
            });
        }
    });

    let (tx, rx) = mpsc::channel::<HotkeyEvent>();
    let staging_for_listener = staging.clone();
    let hotkey_bindings = cfg.hotkey_bindings();
    std::thread::spawn(move || {
        if let Err(e) = source.start(tx, hotkey_bindings) {
            staging_for_listener.error(&format!("全局热键监听启动失败：{e}"));
        }
    });

    std::thread::spawn(move || {
        run_loop(
            runtime,
            injector,
            staging,
            rx,
            current_style,
            wake_out_rx,
            audio_cmd_rx,
            cfg.audio.input_device.clone(),
        );
    });
}

fn run_loop(
    runtime: Arc<Mutex<RuntimeState>>,
    injector: Box<dyn Injector>,
    staging: Staging,
    rx: mpsc::Receiver<HotkeyEvent>,
    current_style: Arc<Mutex<Option<String>>>,
    wake_out_rx: mpsc::Receiver<
        Option<(WakewordConfig, Arc<RingBuffer>, mpsc::Receiver<WakeEvent>)>,
    >,
    audio_rx: mpsc::Receiver<AudioCommand>,
    initial_audio_device: Option<String>,
) {
    let mut recorder = match AudioRecorder::new_with_device(initial_audio_device.as_deref()) {
        Ok(r) => Some(r),
        Err(e) => {
            staging.error(&format!("麦克风初始化失败：{e}"));
            None
        }
    };
    // 设备热切换：录音中到达的命令延后到本次录音结束再执行
    let mut pending_device: Option<Option<String>> = None;

    let mut state = State::Idle;
    // 唤醒词资源（独立于 State enum，由管理线程热更新）
    let mut wake_buffer: Option<Arc<RingBuffer>> = None;
    let mut wake_rx: Option<mpsc::Receiver<WakeEvent>> = None;
    let mut wake_cfg: Option<WakewordConfig> = None;
    let poll_interval = Duration::from_millis(50); // 轮询阈值到期
    let injector: Arc<dyn Injector> = Arc::from(injector); // 指令倒计时线程需要共享 injector
    // 指令代次：每次新录音/新指令 bump，倒计时线程执行前比对，防串台
    let command_gen = Arc::new(AtomicU64::new(0));

    // 返回应当的"空闲"态：唤醒词启用时返回 Listening，否则 Idle。
    let idle_state = |wake_buffer: &Option<Arc<RingBuffer>>,
                      wake_rx: &Option<mpsc::Receiver<WakeEvent>>| {
        if wake_buffer.is_some() && wake_rx.is_some() {
            State::Listening
        } else {
            State::Idle
        }
    };

    loop {
        // 每轮循环取一次运行时快照：设置页保存后可热加载的配置
        // （模型 / 毫秒 / 指令词表）在此生效
        let (backend, cleaner, lexicon, threshold, double_press, command_countdown) = {
            let g = runtime.lock().unwrap();
            (
                g.backend.clone(),
                g.cleaner.clone(),
                g.lexicon.clone(),
                g.threshold,
                g.double_press,
                g.command_countdown,
            )
        };
        // 唤醒词热开关：应用管理线程发来的资源更新（启/停麦克风监听）
        while let Ok(update) = wake_out_rx.try_recv() {
            match update {
                Some((wcfg, buf, wrx)) => {
                    wake_cfg = Some(wcfg);
                    wake_buffer = Some(buf);
                    wake_rx = Some(wrx);
                    if matches!(state, State::Idle) {
                        state = State::Listening;
                    }
                }
                None => {
                    wake_cfg = None;
                    wake_buffer = None;
                    wake_rx = None;
                    if matches!(state, State::Listening) {
                        state = State::Idle;
                    }
                }
            }
        }
        // 录音器设备热切换：空闲时立即重建，录音中延后到结束后再换
        while let Ok(cmd) = audio_rx.try_recv() {
            match cmd {
                AudioCommand::ReloadDevice(name) => {
                    if matches!(state, State::Recording { .. }) {
                        pending_device = Some(name);
                    } else {
                        recorder = match AudioRecorder::new_with_device(name.as_deref()) {
                            Ok(r) => Some(r),
                            Err(e) => {
                                staging.error(&format!("麦克风初始化失败：{e}"));
                                None
                            }
                        };
                    }
                }
            }
        }
        if let Some(name) = pending_device.take() {
            if matches!(state, State::Recording { .. }) {
                pending_device = Some(name); // 本次录音未结束，下轮再试
            } else {
                recorder = match AudioRecorder::new_with_device(name.as_deref()) {
                    Ok(r) => Some(r),
                    Err(e) => {
                        staging.error(&format!("麦克风初始化失败：{e}"));
                        None
                    }
                };
            }
        }
        match rx.recv_timeout(poll_interval) {
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // 按住期间：超阈值才显示暂存条（避免短按一闪而过），然后设"识别中"
                if let State::Recording { started, mode, bar_shown, wake_finish_rx, .. } = &mut state {
                    if started.elapsed() >= threshold {
                        if !*bar_shown {
                            staging.show();
                            *bar_shown = true;
                        }
                        staging.set_busy(true);
                        staging.set_status(mode.recognizing_label());
                    }
                    // 唤醒词录音：轮询 ASR 结果
                    if let Some(ref rx) = wake_finish_rx {
                        if let Ok(result) = rx.try_recv() {
                            handle_wake_result(
                                &staging, &cleaner, &injector, result,
                                *mode, command_countdown, &command_gen,
                                &lexicon, &current_style,
                            );
                            staging.set_recording(false);
                            staging.set_busy(false);
                            state = if wake_buffer.is_some() && wake_rx.is_some() {
                                State::Listening
                            } else {
                                State::Idle
                            };
                            continue;
                        }
                    }
                }
                // PendingCommit 超时未等到第二击 → 确认单击，提交
                if let State::PendingCommit { since } = &state {
                    if since.elapsed() >= double_press {
                        commit(&staging, injector.as_ref());
                        state = idle_state(&wake_buffer, &wake_rx);
                    }
                }
                // Listening 态下轮询唤醒词事件
                if let State::Listening = &state {
                    if let (Some(ref buffer), Some(ref wrx), Some(ref wcfg)) = (
                        wake_buffer.as_ref(),
                        wake_rx.as_ref(),
                        wake_cfg.as_ref(),
                    ) {
                            if let Ok(event) = wrx.try_recv() {
                                let WakeEvent::Detected { word, position } = event;
                                eprintln!(
                                    "[drop-typing] pipeline 收到 WakeEvent: keyword='{}' action='{}'",
                                    word.text, word.action,
                                );
                                match word.action.as_str() {
                                    // 唤醒词 → 立即提交暂存条（不录音）
                                    "commit" => {
                                        commit(&staging, injector.as_ref());
                                    }
                                    // 唤醒词 → 清空暂存条（不录音）
                                    "clear" => {
                                        // 第一唤：仅消除错误，保留暂存条
                                        if staging.has_error() {
                                            staging.clear_error();
                                        } else {
                                            // 第二唤（或无错误）：完整清空
                                            command_gen.fetch_add(1, Ordering::SeqCst);
                                            staging.take();
                                            staging.set_recording(false);
                                            staging.set_busy(false);
                                            staging.set_status("");
                                            staging.set_repair_note("");
                                            staging.clear_command();
                                            staging.clear_error();
                                            staging.hide();
                                        }
                                    }
                                    // 其他 action → 录音转写
                                    _ => {
                                        start_wake_recording(
                                            &staging, &recorder, wcfg, &backend,
                                            buffer, word, position,
                                            &cleaner, &injector, command_countdown,
                                            &command_gen, &lexicon, &current_style,
                                            &mut state,
                                        );
                                    }
                                }
                            }
                        }
                    }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(ev) => match ev {
            HotkeyEvent::Error(msg) => staging.error(&msg),

            HotkeyEvent::CancelDown => {
                // 第一按 ESC：仅消除错误，保留暂存条
                if staging.has_error() {
                    staging.clear_error();
                    state = idle_state(&wake_buffer, &wake_rx);
                    continue;
                }
                // 第二按 ESC（或无错误）：完整清空
                if let State::Recording { .. } = &state {
                    if let Some(r) = &recorder {
                        r.discard();
                    }
                }
                // 取消尚未执行的指令倒计时
                command_gen.fetch_add(1, Ordering::SeqCst);
                staging.take(); // 清空文本（不提交、不粘贴）
                staging.set_recording(false);
                staging.set_busy(false);
                staging.set_status("");
                staging.set_repair_note("");
                staging.clear_command();
                staging.clear_error();
                staging.hide();
                state = if wake_buffer.is_some() && wake_rx.is_some() {
                    State::Listening
                } else {
                    State::Idle
                };
            }

            HotkeyEvent::MouseDoubleClick => {
                // 录音进行中：忽略，不打断
                if matches!(state, State::Recording { .. }) {
                    continue;
                }
                // 异常态：任一确认行为第一次仅消除错误（与确认键语义一致），
                // 不提交、不清文本；无内容时顺带隐藏窗口
                if staging.has_error() {
                    staging.clear_error();
                    if staging.text().trim().is_empty() {
                        staging.hide();
                    }
                    state = idle_state(&wake_buffer, &wake_rx);
                    continue;
                }
                // 暂存条无内容：零副作用
                if staging.text().trim().is_empty() {
                    continue;
                }
                state = idle_state(&wake_buffer, &wake_rx); // 取消 PendingCommit 待定
                // 取消尚未执行的指令倒计时，防止粘贴后倒计时按键打进输入框
                command_gen.fetch_add(1, Ordering::SeqCst);
                // 双击会在目标输入框选中一个单词，先按 → 折叠选区，
                // 避免粘贴把被选词替换掉
                let _ = injector.simulate_combo(&command::KeyCombo {
                    modifiers: vec![],
                    key: "RIGHT".to_string(),
                });
                commit(&staging, injector.as_ref());
            }

            HotkeyEvent::OtherKeyDown => {
                // 仅键盘录音期间其它键按下才 taint；鼠标录音不受键盘干扰
                if let State::Recording { tainted, started_by_mouse, .. } = &mut state {
                    if !*started_by_mouse {
                        *tainted = true;
                    }
                }
            }

            HotkeyEvent::TriggerDown | HotkeyEvent::RepairDown | HotkeyEvent::CommandDown => {
                // 若在 PendingCommit 状态 → 提取第一击时间，进入录音；
                // 若已在录音中 → 根据来源决定行为
                let carry_pending = match &state {
                    State::PendingCommit { since } => Some(*since),
                    State::Recording { started_by_mouse, wake_word, .. } => {
                        // 唤醒词录音中：忽略热键（录音由静音检测结束）
                        if wake_word.is_some() {
                            continue;
                        }
                        // 键盘事件到达鼠标录音 → 忽略，不 taint
                        if *started_by_mouse {
                            continue;
                        }
                        // 键盘事件到达键盘录音 → taint（同一设备二次按下）
                        if let State::Recording { tainted, .. } = &mut state {
                            *tainted = true;
                        }
                        continue;
                    }
                    State::Idle => None,
                    State::Listening => None,
                };

                let mode = match ev {
                    HotkeyEvent::RepairDown => RecordMode::Repair,
                    HotkeyEvent::CommandDown => RecordMode::Command,
                    _ => RecordMode::Input,
                };

                let Some(r) = &recorder else {
                    staging.error("录音器不可用（麦克风初始化失败）");
                    continue;
                };

                // 任何新录音开始都取消尚未执行的指令倒计时
                command_gen.fetch_add(1, Ordering::SeqCst);
                // 异常态下按确认键（录入通道）：标记本次按下仅用于消除错误
                // （须在 clear_error 之前判定）
                let dismiss_only =
                    matches!(ev, HotkeyEvent::TriggerDown) && staging.has_error();
                staging.clear_error();
                staging.partial("");
                staging.set_repair_note(""); // 清除上次修正的修复意见
                staging.clear_command(); // 清除上次指令的展示/倒计时
                // 暂不 show：等超时判定为长按后才显示，避免短按瞬间闪现

                // 创建 PCM 通道，录音立即开始
                let (pcm_tx, pcm_rx) = mpsc::channel::<Vec<u8>>();
                let session = None;
                let mut pending_rx = None;

                if let Some(b) = &backend {
                    if let AsrBackend::Realtime(p) = b.as_ref() {
                        let (ptx, prx) = mpsc::channel::<String>();
                        let st = staging.clone();
                        std::thread::spawn(move || {
                            for text in prx {
                                match mode {
                                    RecordMode::Input | RecordMode::Command => st.partial(&text),
                                    // 修复模式：直接走 repair-note 独立元素（特殊背景色）
                                    RecordMode::Repair => st.set_repair_note(&text),
                                }
                            }
                        });

                        // 音频转发器：缓冲录音数据，等会话就绪后补发 + 续传
                        let (fwd_tx, fwd_rx) = mpsc::channel::<Arc<dyn RealtimeSession>>();
                        std::thread::spawn(move || {
                            let mut buf: Vec<Vec<u8>> = Vec::new();
                            let mut sess: Option<Arc<dyn RealtimeSession>> = None;
                            loop {
                                if sess.is_none() {
                                    if let Ok(s) = fwd_rx.try_recv() {
                                        for chunk in buf.drain(..) {
                                            if s.send_audio(&chunk).is_err() {
                                                return;
                                            }
                                        }
                                        sess = Some(s);
                                    }
                                }
                                match pcm_rx.recv() {
                                    Ok(chunk) => {
                                        if let Some(ref s) = sess {
                                            if s.send_audio(&chunk).is_err() {
                                                return;
                                            }
                                        } else {
                                            buf.push(chunk);
                                        }
                                    }
                                    Err(_) => return,
                                }
                            }
                        });

                        // 后台建连，不阻塞事件循环
                        let p = Arc::clone(p);
                        let (sess_tx, sess_rx) = mpsc::channel();
                        std::thread::spawn(move || {
                            let result: anyhow::Result<Arc<dyn RealtimeSession>> =
                                p.start_session(ptx).map(|s| Arc::from(s));
                            if let Ok(ref s) = result {
                                let _ = fwd_tx.send(s.clone());
                            }
                            let _ = sess_tx.send(result);
                        });
                        pending_rx = Some(sess_rx);
                    }
                }

                if let Err(e) = r.start(Some(pcm_tx)) {
                    staging.error(&format!("录音启动失败：{e}"));
                    continue;
                }
                staging.set_recording(true);
                state = State::Recording {
                    started: Instant::now(),
                    tainted: false,
                    mode,
                    bar_shown: false,
                    dismiss_only,
                    pending_since: carry_pending,
                    session,
                    pending_rx,
                    started_by_mouse: false,  // 键盘启动
                    wake_word: None,
                    wake_finish_rx: None,
                };
            }

            HotkeyEvent::MouseTriggerDown | HotkeyEvent::MouseRepairDown => {
                    // 鼠标侧键事件。若已在键盘录音中 → 忽略，不 taint
                    let carry_pending = match &state {
                        State::PendingCommit { since } => Some(*since),
                        State::Recording { started_by_mouse, wake_word, .. } => {
                            // 唤醒词录音中：忽略鼠标事件
                            if wake_word.is_some() {
                                continue;
                            }
                            if !*started_by_mouse {
                                // 鼠标事件到达键盘录音 → 忽略
                                continue;
                            }
                            // 鼠标事件到达鼠标录音 → taint（同一设备二次按下）
                            if let State::Recording { tainted, .. } = &mut state {
                                *tainted = true;
                            }
                            continue;
                        }
                        State::Idle => None,
                        State::Listening => None,
                    };

                    // 鼠标不支持 command 通道，RepairDown → RecordMode::Repair，
                    // TriggerDown → RecordMode::Input
                    let mode = if matches!(ev, HotkeyEvent::MouseRepairDown) {
                        RecordMode::Repair
                    } else {
                        RecordMode::Input
                    };

                    let Some(r) = &recorder else {
                        staging.error("录音器不可用（麦克风初始化失败）");
                        continue;
                    };

                    command_gen.fetch_add(1, Ordering::SeqCst);
                    let dismiss_only =
                        matches!(ev, HotkeyEvent::MouseTriggerDown) && staging.has_error();
                    staging.clear_error();
                    staging.partial("");
                    staging.set_repair_note("");
                    staging.clear_command();

                    let (pcm_tx, pcm_rx) = mpsc::channel::<Vec<u8>>();
                    let session = None;
                    let mut pending_rx = None;

                    if let Some(b) = &backend {
                        if let AsrBackend::Realtime(p) = b.as_ref() {
                            let (ptx, prx) = mpsc::channel::<String>();
                            let st = staging.clone();
                            std::thread::spawn(move || {
                                for text in prx {
                                    match mode {
                                        RecordMode::Input | RecordMode::Command => st.partial(&text),
                                        RecordMode::Repair => st.set_repair_note(&text),
                                    }
                                }
                            });

                            let (fwd_tx, fwd_rx) = mpsc::channel::<Arc<dyn RealtimeSession>>();
                            std::thread::spawn(move || {
                                let mut buf: Vec<Vec<u8>> = Vec::new();
                                let mut sess: Option<Arc<dyn RealtimeSession>> = None;
                                loop {
                                    if sess.is_none() {
                                        if let Ok(s) = fwd_rx.try_recv() {
                                            for chunk in buf.drain(..) {
                                                if s.send_audio(&chunk).is_err() {
                                                    return;
                                                }
                                            }
                                            sess = Some(s);
                                        }
                                    }
                                    match pcm_rx.recv() {
                                        Ok(chunk) => {
                                            if let Some(ref s) = sess {
                                                if s.send_audio(&chunk).is_err() {
                                                    return;
                                                }
                                            } else {
                                                buf.push(chunk);
                                            }
                                        }
                                        Err(_) => return,
                                    }
                                }
                            });

                            let p = Arc::clone(p);
                            let (sess_tx, sess_rx) = mpsc::channel();
                            std::thread::spawn(move || {
                                let result: anyhow::Result<Arc<dyn RealtimeSession>> =
                                    p.start_session(ptx).map(|s| Arc::from(s));
                                if let Ok(ref s) = result {
                                    let _ = fwd_tx.send(s.clone());
                                }
                                let _ = sess_tx.send(result);
                            });
                            pending_rx = Some(sess_rx);
                        }
                    }

                    if let Err(e) = r.start(Some(pcm_tx)) {
                        staging.error(&format!("录音启动失败：{e}"));
                        continue;
                    }
                    staging.set_recording(true);
                    state = State::Recording {
                        started: Instant::now(),
                        tainted: false,
                        mode,
                        bar_shown: false,
                        dismiss_only,
                        pending_since: carry_pending,
                        session,
                        pending_rx,
                        started_by_mouse: true,  // 鼠标启动
                        wake_word: None,
                    wake_finish_rx: None,
                    };
                }

                HotkeyEvent::MouseTriggerUp | HotkeyEvent::MouseRepairUp => {
                // 鼠标侧键松开：与键盘松开处理逻辑相同（长按 → ASR，短按 → 提交/忽略）
                let State::Recording {
                    started,
                    tainted,
                    mode,
                    bar_shown: _,
                    dismiss_only,
                    pending_since,
                    session,
                    pending_rx,
                    wake_word: _,
                    wake_finish_rx: _,
                    started_by_mouse: _,
                } = state
                else {
                    continue;
                };
                state = idle_state(&wake_buffer, &wake_rx);
                staging.set_recording(false);

                let Some(r) = &recorder else { continue };
                let duration = started.elapsed();

                let mut session = session;
                let mut session_err: Option<String> = None;
                if duration >= threshold {
                    if let Some(rx) = pending_rx {
                        match rx.try_recv() {
                            Ok(Ok(s)) => session = Some(s),
                            Ok(Err(e)) => session_err = Some(format!("{e:#}")),
                            Err(_) => session_err = Some("ASR 会话建立超时".into()),
                        }
                    }
                } else {
                    drop(pending_rx);
                }

                if tainted {
                    r.discard();
                    staging.set_status("");
                    staging.set_repair_note("");
                    staging.hide();
                } else if duration < threshold {
                    r.discard();
                    staging.set_status("");
                    match mode {
                        RecordMode::Input => {
                            if dismiss_only {
                                staging.clear_error();
                                if staging.text().trim().is_empty() {
                                    staging.hide();
                                }
                                continue;
                            }
                            let text = staging.text();
                            if text.trim().is_empty() {
                                staging.hide();
                            } else if let Some(since) = pending_since {
                                if since.elapsed() < double_press {
                                    staging.take();
                                } else {
                                    state = State::PendingCommit { since: Instant::now() };
                                }
                            } else {
                                state = State::PendingCommit { since: Instant::now() };
                            }
                        }
                        RecordMode::Repair | RecordMode::Command => {
                            staging.set_repair_note("");
                            staging.hide();
                        }
                    }
                } else {
                    // 长按：与键盘逻辑完全相同
                    match (&backend, session) {
                        (Some(b), Some(s)) if matches!(b.as_ref(), AsrBackend::Realtime(_)) => {
                            r.discard();
                            staging.set_busy(true);
                            staging.set_status(mode.recognizing_label());
                            let result = s.finish();
                            staging.set_busy(false);
                            match result {
                                Ok(text) if !text.trim().is_empty() => {
                                    match mode {
                                        RecordMode::Input => {
                                            let pc = prompts::load_prompts();
                                            let prompt = {
                                                let style = current_style.lock().unwrap().clone();
                                                prompts::effective_clean_prompt(&pc, style.as_deref())
                                            };
                                            clean_and_append(&staging, &cleaner, text.trim(), &prompt)
                                        }
                                        RecordMode::Repair => {
                                            repair_and_replace(&staging, &cleaner, text.trim())
                                        }
                                        RecordMode::Command => run_command(
                                            &staging,
                                            &injector,
                                            text.trim(),
                                            command_countdown,
                                            &command_gen,
                                            &lexicon,
                                        ),
                                    }
                                }
                                Ok(_) => {
                                    staging.set_status("");
                                    staging.error("ASR 返回空文本")
                                }
                                Err(e) => {
                                    staging.set_status("");
                                    staging.error(&format!("ASR 失败：{e}"))
                                }
                            }
                        }
                        (Some(b), None) if matches!(b.as_ref(), AsrBackend::Realtime(_)) => {
                            r.discard();
                            staging.set_status("");
                            if let Some(e) = session_err {
                                staging.error(&format!("ASR 会话建立失败：{e}"));
                            } else {
                                staging.error("ASR 会话未建立");
                            }
                        }
                        (Some(b), _) => {
                            let AsrBackend::Batch(p) = b.as_ref() else {
                                unreachable!()
                            };
                            match r.stop() {
                                Ok(wav) => {
                                    let pc = prompts::load_prompts();
                                    let prompt = {
                                        let style = current_style.lock().unwrap().clone();
                                        prompts::effective_clean_prompt(&pc, style.as_deref())
                                    };
                                    spawn_transcribe(
                                    &staging, p.clone(), &cleaner, mode, wav,
                                    &injector, command_countdown, &command_gen, lexicon.clone(),
                                    prompt,
                                )}
                                Err(e) => {
                                    staging.set_status("");
                                    staging.error(&format!("录音失败：{e}"))
                                }
                            }
                        }
                        (None, _) => {
                            r.discard();
                            staging.set_status("");
                            staging.error("未配置 ASR API Key，无法转写。");
                        }
                    }
                }
            }

            HotkeyEvent::TriggerUp | HotkeyEvent::RepairUp | HotkeyEvent::CommandUp => {
                let State::Recording {
                    started,
                    tainted,
                    mode,
                    bar_shown: _,
                    dismiss_only,
                    pending_since,
                    session,
                    pending_rx,
                    wake_word: _,
                    wake_finish_rx: _,
                    started_by_mouse: _,
                } = state
                else {
                    continue;
                };
                state = idle_state(&wake_buffer, &wake_rx);
                staging.set_recording(false);

                let Some(r) = &recorder else { continue };
                let duration = started.elapsed();

                // 尝试从后台建连取回会话（短按直接丢弃，不等待）
                let mut session = session;
                let mut session_err: Option<String> = None;
                if duration >= threshold {
                    if let Some(rx) = pending_rx {
                        match rx.try_recv() {
                            Ok(Ok(s)) => session = Some(s),
                            Ok(Err(e)) => session_err = Some(format!("{e:#}")),
                            Err(_) => session_err = Some("ASR 会话建立超时".into()),
                        }
                    }
                } else {
                    drop(pending_rx);
                }

                if tainted {
                    // 修饰键被用作组合键（如 ⌘Space 或双修饰键同时按下），作废
                    r.discard();
                    staging.set_status("");
                    staging.set_repair_note("");
                    staging.hide();
                } else if duration < threshold {
                    // 短按
                    r.discard();
                    staging.set_status("");
                    match mode {
                        RecordMode::Input => {
                            if dismiss_only {
                                // 按下时处于异常态：短按仅消除错误，不提交、不清文本；
                                // 无内容时顺带隐藏窗口
                                staging.clear_error();
                                if staging.text().trim().is_empty() {
                                    staging.hide();
                                }
                                continue;
                            }
                            let text = staging.text();
                            if text.trim().is_empty() {
                                // 暂存条为空，直接隐藏
                                staging.hide();
                            } else if let Some(since) = pending_since {
                                // PendingCommit 触发的录音 → 检查是否在双击窗口内
                                if since.elapsed() < double_press {
                                    // 双击：清空暂存条（不提交），暂存条保持显示
                                    staging.take();
                                } else {
                                    // 窗口已过，视为新一次单击 → 重新进入待定
                                    state = State::PendingCommit { since: Instant::now() };
                                }
                            } else {
                                // 第一次短按 → 进入待定状态
                                state = State::PendingCommit { since: Instant::now() };
                            }
                        }
                        // 右 ⌥ / 右 ⇧ 短按无动作（PRD 第 5 节）
                        RecordMode::Repair | RecordMode::Command => {
                            staging.set_repair_note("");
                            staging.hide();
                        }
                    }
                } else {
                    // 长按：停止录音 → ASR → 按 mode 分发
                    match (&backend, session) {
                        (Some(b), Some(s)) if matches!(b.as_ref(), AsrBackend::Realtime(_)) => {
                            r.discard(); // 实时路径不需要本地 WAV
                            staging.set_busy(true);
                            staging.set_status(mode.recognizing_label());
                            let result = s.finish();
                            staging.set_busy(false);
                            match result {
                                Ok(text) if !text.trim().is_empty() => {
                                    match mode {
                                        RecordMode::Input => {
                                            let pc = prompts::load_prompts();
                                            let prompt = {
                                                let style = current_style.lock().unwrap().clone();
                                                prompts::effective_clean_prompt(&pc, style.as_deref())
                                            };
                                            clean_and_append(&staging, &cleaner, text.trim(), &prompt)
                                        }
                                        RecordMode::Repair => {
                                            repair_and_replace(&staging, &cleaner, text.trim())
                                        }
                                        RecordMode::Command => run_command(
                                            &staging,
                                            &injector,
                                            text.trim(),
                                            command_countdown,
                                            &command_gen,
                                            &lexicon,
                                        ),
                                    }
                                }
                                Ok(_) => {
                                    staging.set_status("");
                                    staging.error("ASR 返回空文本")
                                }
                                Err(e) => {
                                    staging.set_status("");
                                    staging.error(&format!("ASR 失败：{e}"))
                                }
                            }
                        }
                        (Some(b), None) if matches!(b.as_ref(), AsrBackend::Realtime(_)) => {
                            r.discard();
                            staging.set_status("");
                            if let Some(e) = session_err {
                                staging.error(&format!("ASR 会话建立失败：{e}"));
                            } else {
                                staging.error("ASR 会话未建立");
                            }
                        }
                        (Some(b), _) => {
                            let AsrBackend::Batch(p) = b.as_ref() else {
                                unreachable!()
                            };
                            match r.stop() {
                                Ok(wav) => {
                                    let pc = prompts::load_prompts();
                                    let prompt = {
                                        let style = current_style.lock().unwrap().clone();
                                        prompts::effective_clean_prompt(&pc, style.as_deref())
                                    };
                                    spawn_transcribe(
                                    &staging, p.clone(), &cleaner, mode, wav,
                                    &injector, command_countdown, &command_gen, lexicon.clone(),
                                    prompt,
                                )}
                                Err(e) => {
                                    staging.set_status("");
                                    staging.error(&format!("录音失败：{e}"))
                                }
                            }
                        }
                        (None, _) => {
                            r.discard();
                            staging.set_status("");
                            staging.error("未配置 ASR API Key，无法转写。");
                        }
                    }
                }
            }
            }, // match ev
        } // match recv_timeout
    } // loop
}

// ── 唤醒词 → RecordMode 映射 ────────────────────────────────────────

fn wake_word_to_mode(word: &WakeWord) -> RecordMode {
    match word.action.as_str() {
        "input" => RecordMode::Input,
        "repair" => RecordMode::Repair,
        "command" => RecordMode::Command,
        other => {
            eprintln!(
                "[drop-typing] 唤醒词 '{}' 的 action='{other}' 未知，回退为 Input",
                word.text,
            );
            RecordMode::Input
        }
    }
}

// ── 唤醒词触发录音 ────────────────────────────────────────────────────

/// 唤醒词检测到后：从 RingBuffer 中裁取音频喂给 ASR。
///
/// 创建 TailReader 从 `position - pre_roll_samples` 开始读取，
/// 在后台线程中持续读取 PCM → 送入 RealtimeSession → 静音检测 → finish。
#[allow(clippy::too_many_arguments)]
fn start_wake_recording(
    staging: &Staging,
    _recorder: &Option<AudioRecorder>,
    wake_cfg: &WakewordConfig,
    backend: &Option<Arc<AsrBackend>>,
    buffer: &Arc<RingBuffer>,
    word: WakeWord,
    position: u64,
    _cleaner: &Option<Arc<dyn TextCleaner>>,
    _injector: &Arc<dyn Injector>,
    _command_countdown: Duration,
    _command_gen: &Arc<AtomicU64>,
    _lexicon: &Arc<command::Lexicon>,
    _current_style: &Arc<Mutex<Option<String>>>,
    state: &mut State,
) {
    let mode = wake_word_to_mode(&word);
    eprintln!(
        "[drop-typing] start_wake_recording 进入：keyword='{}' action='{}' mode={:?}",
        word.text, word.action, mode,
    );
    let sample_rate: u64 = 16_000;
    let pre_roll_samples = wake_cfg.pre_roll_ms * sample_rate / 1000;
    let silence_samples = wake_cfg.silence_timeout_ms * sample_rate / 1000;
    // 唤醒词自身的采样数
    let _wake_word_samples = word.duration_ms() * sample_rate / 1000;

    // 从唤醒词结束位置往前 pre_roll 开始读（唤醒词之前的上下文 + 唤醒词后的内容）
    let read_from = position.saturating_sub(pre_roll_samples);

    let backend = match backend {
        Some(b) => b.clone(),
        None => {
            staging.error("未配置 ASR API Key，无法转写。");
            return;
        }
    };

    // 仅支持实时后端（批量路径较复杂，Phase 1 暂不处理）
    let provider = match backend.as_ref() {
        AsrBackend::Realtime(p) => p.clone(),
        _ => {
            staging.error("唤醒词仅支持实时 ASR 后端。");
            return;
        }
    };

    // 清除上次录音的中间结果
    staging.partial("");
    staging.set_repair_note("");
    staging.clear_command();
    staging.clear_error();

    // 显示暂存条 + 状态徽章
    staging.show();
    let display = word.display_name();
    staging.set_status(&format!("{display} ✓"));

    // 创建 PCM 通道（TailReader → PCM → ASR session）
    let (pcm_tx, pcm_rx) = mpsc::channel::<Vec<u8>>();

    // 部分结果回调
    let (ptx, prx) = mpsc::channel::<String>();
    let st = staging.clone();
    let wake_mode = mode;
    std::thread::spawn(move || {
        for text in prx {
            match wake_mode {
                RecordMode::Input | RecordMode::Command => st.partial(&text),
                RecordMode::Repair => st.set_repair_note(&text),
            }
        }
    });

    // 音频转发器：缓冲 TailReader 数据，等会话就绪后补发 + 续传
    let (fwd_tx, fwd_rx) = mpsc::channel::<Arc<dyn RealtimeSession>>();
    std::thread::spawn(move || {
        let mut buf: Vec<Vec<u8>> = Vec::new();
        let mut sess: Option<Arc<dyn RealtimeSession>> = None;
        loop {
            if sess.is_none() {
                if let Ok(s) = fwd_rx.try_recv() {
                    for chunk in buf.drain(..) {
                        if s.send_audio(&chunk).is_err() {
                            return;
                        }
                    }
                    sess = Some(s);
                }
            }
            match pcm_rx.recv() {
                Ok(chunk) => {
                    if let Some(ref s) = sess {
                        if s.send_audio(&chunk).is_err() {
                            return;
                        }
                    } else {
                        buf.push(chunk);
                    }
                }
                Err(_) => return,
            }
        }
    });

    // 后台建连
    let (sess_tx, sess_rx) = mpsc::channel();
    {
        let p = provider.clone();
        std::thread::spawn(move || {
            let result: anyhow::Result<Arc<dyn RealtimeSession>> =
                p.start_session(ptx).map(|s| Arc::from(s));
            if let Ok(ref s) = result {
                let _ = fwd_tx.send(s.clone());
            }
            let _ = sess_tx.send(result);
        });
    }

    // TailReader → PCM 线程（含静音检测）
    let mut tail = TailReader::new(buffer.clone(), read_from);
    let (finish_tx, finish_rx) = mpsc::channel::<anyhow::Result<String>>();
    std::thread::Builder::new()
        .name("drop-typing-wake-audio".into())
        .spawn(move || {
            let mut silence_count: u64 = 0;
            // 静音能量阈值（RMS）：经验值，f32 归一化后的典型静音 RMS < 0.01
            const SILENCE_RMS_THRESHOLD: f32 = 0.02;
            let silence_max = silence_samples;

            loop {
                let samples = tail.read_available();
                if !samples.is_empty() {
                    // 计算 RMS
                    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
                    let rms = (sum_sq / samples.len() as f32).sqrt();

                    // f32 → s16le PCM
                    let mut bytes = Vec::with_capacity(samples.len() * 2);
                    for s in &samples {
                        let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                        bytes.extend_from_slice(&v.to_le_bytes());
                    }

                    if pcm_tx.send(bytes).is_err() {
                        // 接收端已关闭（session 结束）
                        break;
                    }

                    if rms < SILENCE_RMS_THRESHOLD {
                        silence_count += samples.len() as u64;
                    } else {
                        silence_count = 0;
                    }
                }

                if silence_count >= silence_max {
                    break;
                }

                std::thread::sleep(std::time::Duration::from_millis(40));
            }

            drop(pcm_tx); // 通知音频转发器结束
            // 等待 session 完成（通过 sess_rx），然后 finish
            let result = match sess_rx.recv() {
                Ok(Ok(session)) => session.finish(),
                Ok(Err(e)) => Err(anyhow::anyhow!("ASR 会话建立失败：{e:#}")),
                Err(_) => Err(anyhow::anyhow!("ASR 会话建立超时")),
            };
            let _ = finish_tx.send(result);
        })
        .expect("启动唤醒词音频线程失败");

    staging.set_recording(true);
    staging.set_busy(true);
    staging.set_status(mode.recognizing_label());

    *state = State::Recording {
        started: Instant::now(),
        tainted: false,
        mode,
        bar_shown: true,
        dismiss_only: false,
        pending_since: None,
        session: None,
        pending_rx: None,
        started_by_mouse: false,
        wake_word: Some(word),
        wake_finish_rx: Some(finish_rx),
    };
}

/// 处理唤醒词录音的 ASR 结果（与热键松手路径相同）。
fn handle_wake_result(
    staging: &Staging,
    cleaner: &Option<Arc<dyn TextCleaner>>,
    injector: &Arc<dyn Injector>,
    result: anyhow::Result<String>,
    mode: RecordMode,
    command_countdown: Duration,
    command_gen: &Arc<AtomicU64>,
    lexicon: &Arc<command::Lexicon>,
    current_style: &Arc<Mutex<Option<String>>>,
) {
    match result {
        Ok(text) if !text.trim().is_empty() => {
            match mode {
                RecordMode::Input => {
                    let pc = prompts::load_prompts();
                    let prompt = {
                        let style = current_style.lock().unwrap().clone();
                        prompts::effective_clean_prompt(&pc, style.as_deref())
                    };
                    clean_and_append(staging, cleaner, text.trim(), &prompt)
                }
                RecordMode::Repair => {
                    repair_and_replace(staging, cleaner, text.trim())
                }
                RecordMode::Command => run_command(
                    staging,
                    injector,
                    text.trim(),
                    command_countdown,
                    command_gen,
                    lexicon,
                ),
            }
        }
        Ok(_) => {
            staging.set_status("");
            staging.error("ASR 返回空文本")
        }
        Err(e) => {
            staging.set_status("");
            staging.error(&format!("ASR 失败：{e}"))
        }
    }
}

/// 短按提交：暂存条 → 剪贴板 → Cmd+V → 恢复剪贴板 → 清空暂存条
fn commit(staging: &Staging, injector: &dyn Injector) {
    let text = staging.take();
    if text.trim().is_empty() {
        staging.hide();
        return;
    }
    match injector.paste_text(&text) {
        Ok(()) => {
            staging.committed();
            staging.hide();
        }
        Err(e) => {
            // 提交失败不丢内容：回滚到暂存条，窗口保持可见
            staging.set_text(&text);
            staging.error(&format!("提交失败（内容已保留在暂存条）：{e}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_state_defaults() {
        let st = RuntimeState::from_config(&Config::default());
        assert!(st.backend.is_none());
        assert!(st.cleaner.is_none());
        assert_eq!(st.threshold, Duration::from_millis(150));
        assert_eq!(st.double_press, Duration::from_millis(350));
        assert_eq!(st.command_countdown, Duration::from_millis(2000));
        assert!(st.lexicon.entry_count() > 0);
    }

    #[test]
    fn runtime_state_reflects_custom_thresholds() {
        let mut cfg = Config::default();
        cfg.long_press_threshold_ms = 500;
        cfg.double_press_window_ms = 400;
        cfg.command_countdown_ms = 3000;
        let st = RuntimeState::from_config(&cfg);
        assert_eq!(st.threshold, Duration::from_millis(500));
        assert_eq!(st.double_press, Duration::from_millis(400));
        assert_eq!(st.command_countdown, Duration::from_millis(3000));
    }
}

/// 批量后端：整段 WAV 异步转写
#[allow(clippy::too_many_arguments)]
fn spawn_transcribe(
    staging: &Staging,
    provider: Arc<dyn asr::AsrProvider>,
    cleaner: &Option<Arc<dyn TextCleaner>>,
    mode: RecordMode,
    wav: Vec<u8>,
    injector: &Arc<dyn Injector>,
    command_countdown: Duration,
    command_gen: &Arc<AtomicU64>,
    lexicon: Arc<command::Lexicon>,
    system_prompt: String,
) {
    let staging = staging.clone();
    let cleaner = cleaner.clone();
    let injector = injector.clone();
    let command_gen = command_gen.clone();
    staging.set_busy(true);
    staging.set_status(mode.recognizing_label());
    tauri::async_runtime::spawn(async move {
        let ctx = staging.text();
        let ctx = if ctx.trim().is_empty() { None } else { Some(ctx) };
        let result = provider.transcribe(&wav, ctx.as_deref()).await;
        staging.set_busy(false);
        match result {
            Ok(text) if !text.trim().is_empty() => match mode {
                RecordMode::Input => {
                    clean_and_append(&staging, &cleaner, text.trim(), &system_prompt)
                }
                RecordMode::Repair => {
                    repair_and_replace(&staging, &cleaner, text.trim())
                }
                RecordMode::Command => run_command(
                    &staging,
                    &injector,
                    text.trim(),
                    command_countdown,
                    &command_gen,
                    &lexicon,
                ),
            },
            Ok(_) => {
                staging.set_status("");
                staging.error("ASR 返回空文本")
            }
            Err(e) => {
                staging.set_status("");
                staging.error(&format!("ASR 失败：{e:#}"))
            }
        }
    });
}

/// ASR 结果 →（可选）LLM 清洗 → 追加到暂存条（M2）。
///
/// 未配置清洗层（cleaner 为 None）时 ASR 直出；清洗失败降级为原文追加 +
/// 黄底红字提示，不丢用户内容。润色期间仅显示状态徽章"润色中"，
/// 不作半透明预览——文字可读性优先。
fn clean_and_append(
    staging: &Staging,
    cleaner: &Option<Arc<dyn TextCleaner>>,
    text: &str,
    system_prompt: &str,
) {
    let Some(cleaner) = cleaner else {
        staging.append(text);
        staging.set_status("");
        return;
    };
    let staging = staging.clone();
    let cleaner = cleaner.clone();
    let raw = text.to_string();
    staging.set_busy(true);
    staging.set_status("润色中");
    let prompt = system_prompt.to_string();
    tauri::async_runtime::spawn(async move {
        let user_msg = format!("请改写以下文本：\n---\n{raw}\n---");
        let result = cleaner.clean(&user_msg, &prompt).await;
        staging.set_busy(false);
        staging.set_status("");
        match result {
            Ok(cleaned) if !cleaned.trim().is_empty() => {
                staging.append(cleaned.trim());
            }
            Ok(_) => {
                staging.append(&raw);
                staging.error("清洗返回空文本，已直出原文");
            }
            Err(e) => {
                staging.append(&raw);
                staging.error(&format!("清洗失败，已直出原文：{e:#}"));
            }
        }
    });
}

/// 修正通道：ASR 转写的修正指令 + 暂存条当前全文 → LLM 修正 → 整体替换暂存条（M2）。
///
/// 流程：先把修正指令通过 repair-note 独立元素展示（特殊背景色），
/// 然后异步调 LLM repair；成功则 `staging.replace(corrected)` + 清除 repair-note，
/// 失败保留原文 + 黄底红字提示。未配置 LLM 时直接显示错误。
fn repair_and_replace(
    staging: &Staging,
    cleaner: &Option<Arc<dyn TextCleaner>>,
    instruction: &str,
) {
    let original = staging.text();
    if original.trim().is_empty() {
        staging.error("暂存条为空，无法修正");
        return;
    }
    let Some(cleaner) = cleaner else {
        staging.error("未配置 LLM，无法使用语音修正。请在 [llm] 中配置 API Key。");
        return;
    };

    // ★ 通过独立 repair-note 元素展示修正指令（特殊背景色，与正文分离）
    staging.set_repair_note(instruction);

    staging.set_busy(true);
    staging.set_status("修复中");
    let staging = staging.clone();
    let cleaner = cleaner.clone();
    let instruction = instruction.to_string();
    tauri::async_runtime::spawn(async move {
        let result = cleaner.repair(&original, &instruction).await;
        staging.set_busy(false);
        staging.set_status("");
        match result {
            Ok(corrected) if !corrected.trim().is_empty() => {
                staging.replace(corrected.trim());
                staging.set_repair_note(""); // 修正成功，清除修复意见
            }
            Ok(_) => {
                staging.error("修正返回空文本，已保留原文");
            }
            Err(e) => {
                staging.error(&format!("修正失败，已保留原文：{e:#}"));
            }
        }
    });
}

/// 指令通道（M4）：ASR 文本 → 本地解析 → 暂存条大字展示 + 右侧秒级倒计时 → 自动模拟按键。
///
/// 倒计时期间用户按下任意一个右修饰键（开始新录音）即作废本次指令：
/// 通过 `gen` 代次比对实现（Down 事件会 bump 代次）。
fn run_command(
    staging: &Staging,
    injector: &Arc<dyn Injector>,
    text: &str,
    countdown: Duration,
    gen: &Arc<AtomicU64>,
    lexicon: &command::Lexicon,
) {
    staging.set_status("");
    let combo = match command::parse(text, lexicon) {
        Some(command::ParsedCommand::Combo(c)) => c,
        Some(command::ParsedCommand::Script(path)) => {
            staging.error(&format!(
                "指令关联了脚本「{path}」，脚本执行功能即将上线（当前未执行任何操作）"
            ));
            return;
        }
        None => {
            staging.error(&format!("未识别到按键指令：{text}"));
            return;
        }
    };

    // 倒计时秒数（不足 1 秒按 1 秒计；配 0 则立即执行）
    let mut remaining = (countdown.as_millis() as u64 + 999) / 1000;
    staging.show_command(&combo.display(), remaining);

    let staging = staging.clone();
    let injector = injector.clone();
    let gen = gen.clone();
    let my_gen = gen.load(Ordering::SeqCst);
    std::thread::spawn(move || {
        while remaining > 0 {
            std::thread::sleep(Duration::from_secs(1));
            remaining -= 1;
            // 期间用户开始了新录音/新指令 → 放弃执行
            if gen.load(Ordering::SeqCst) != my_gen {
                return;
            }
            staging.command_tick(remaining);
        }
        match injector.simulate_combo(&combo) {
            Ok(()) => {
                staging.committed();
                // 短暂停留让用户看到"已执行"反馈，再清除指令展示并隐藏
                std::thread::sleep(Duration::from_millis(600));
                if gen.load(Ordering::SeqCst) == my_gen {
                    staging.clear_command();
                    staging.hide();
                }
            }
            Err(e) => {
                staging.clear_command();
                staging.error(&format!("按键模拟失败：{e:#}"));
            }
        }
    });
}
