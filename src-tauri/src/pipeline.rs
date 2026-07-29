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
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow;
use tauri::{AppHandle, Listener};

use crate::asr::{self, AsrBackend, RealtimeSession};
use crate::audio::AudioRecorder;
use crate::command;
use crate::config::Config;
use crate::hotkey::{self, HotkeyEvent};
use crate::inject::{self, Injector};
use crate::llm::{self, TextCleaner};
use crate::staging::Staging;

/// 录音目的：输入通道（右 ⌘）、修正通道（右 ⌥）还是指令通道（右 ⇧，M4）。
#[derive(Clone, Copy, PartialEq, Eq)]
enum RecordMode {
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
    Recording {
        started: Instant,
        tainted: bool,
        mode: RecordMode,
        /// 暂存条是否已显示（按下后等阈值到期才 show，避免短按一闪而过）
        bar_shown: bool,
        /// 若本次录音是从 PendingCommit 触发的，携带第一击时间用于双击判定
        pending_since: Option<Instant>,
        /// 实时后端的活动会话（批量后端为 None）
        session: Option<Arc<dyn RealtimeSession>>,
        /// 后台建连中：会话尚未就绪时暂存 Receiver，松手时取回
        pending_rx: Option<mpsc::Receiver<anyhow::Result<Arc<dyn RealtimeSession>>>>,
    },
    /// 输入通道短按后等待判定：超时则单击提交，窗口内再次短按则双击清空
    PendingCommit { since: Instant },
}

pub fn start(app: AppHandle) {
    let staging = Staging::new(app.clone());
    let (cfg, warning) = Config::load_lenient();
    let backend = asr::backend_from_config(&cfg);
    let cleaner = llm::cleaner_from_config(&cfg);
    let injector = inject::default_injector(app.clone());
    let source = hotkey::default_source();
    let lexicon = Arc::new(command::Lexicon::build(Some(&cfg.command)));

    // 启动诊断：配置 / 权限问题直接以黄底红字显示在暂存条
    if let Some(w) = warning {
        staging.error(&w);
    } else if backend.is_none() {
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

    let (tx, rx) = mpsc::channel::<HotkeyEvent>();
    let staging_for_listener = staging.clone();
    std::thread::spawn(move || {
        if let Err(e) = source.start(tx) {
            staging_for_listener.error(&format!("全局热键监听启动失败：{e}"));
        }
    });

    std::thread::spawn(move || {
        run_loop(cfg, backend, cleaner, injector, staging, rx, lexicon);
    });
}

fn run_loop(
    cfg: Config,
    backend: Option<AsrBackend>,
    cleaner: Option<Arc<dyn TextCleaner>>,
    injector: Box<dyn Injector>,
    staging: Staging,
    rx: mpsc::Receiver<HotkeyEvent>,
    lexicon: Arc<command::Lexicon>,
) {
    let recorder = match AudioRecorder::new() {
        Ok(r) => Some(r),
        Err(e) => {
            staging.error(&format!("麦克风初始化失败：{e}"));
            None
        }
    };
    let backend = backend.map(Arc::new);

    let mut state = State::Idle;
    let threshold = Duration::from_millis(cfg.long_press_threshold_ms);
    let strength = llm::Strength::from_config(&cfg.llm_strength());
    let poll_interval = Duration::from_millis(50); // 轮询阈值到期
    let double_press = Duration::from_millis(cfg.double_press_window_ms); // 双击窗口（可配置）
    let command_countdown = Duration::from_millis(cfg.effective_command_countdown_ms()); // 指令确认倒计时（M4）
    let injector: Arc<dyn Injector> = Arc::from(injector); // 指令倒计时线程需要共享 injector
    // 指令代次：每次新录音/新指令 bump，倒计时线程执行前比对，防串台
    let command_gen = Arc::new(AtomicU64::new(0));

    loop {
        match rx.recv_timeout(poll_interval) {
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // 按住期间：超阈值才显示暂存条（避免短按一闪而过），然后设"识别中"
                if let State::Recording { started, mode, bar_shown, .. } = &mut state {
                    if started.elapsed() >= threshold {
                        if !*bar_shown {
                            staging.show();
                            *bar_shown = true;
                        }
                        staging.set_busy(true);
                        staging.set_status(mode.recognizing_label());
                    }
                }
                // PendingCommit 超时未等到第二击 → 确认单击，提交
                if let State::PendingCommit { since } = &state {
                    if since.elapsed() >= double_press {
                        commit(&staging, injector.as_ref());
                        state = State::Idle;
                    }
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(ev) => match ev {
            HotkeyEvent::Error(msg) => staging.error(&msg),

            HotkeyEvent::CancelDown => {
                // Esc 按下：丢弃当前录音（如果有）、清空暂存条并隐藏窗口
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
                state = State::Idle;
            }

            HotkeyEvent::OtherKeyDown => {
                if let State::Recording { tainted, .. } = &mut state {
                    *tainted = true;
                }
            }

            HotkeyEvent::TriggerDown | HotkeyEvent::RepairDown | HotkeyEvent::CommandDown => {
                // 若在 PendingCommit 状态 → 提取第一击时间，进入录音；
                // 若已在录音中 → 另一个修饰键按下，taint
                let carry_pending = match &state {
                    State::PendingCommit { since } => Some(*since),
                    State::Recording { .. } => {
                        if let State::Recording { tainted, .. } = &mut state {
                            *tainted = true;
                        }
                        continue;
                    }
                    State::Idle => None,
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
                    pending_since: carry_pending,
                    session,
                    pending_rx,
                };
            }

            HotkeyEvent::TriggerUp | HotkeyEvent::RepairUp | HotkeyEvent::CommandUp => {
                let State::Recording {
                    started,
                    tainted,
                    mode,
                    bar_shown: _,
                    pending_since,
                    session,
                    pending_rx,
                } = state
                else {
                    continue;
                };
                state = State::Idle;
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
                                            clean_and_append(&staging, &cleaner, text.trim(), strength)
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
                                Ok(wav) => spawn_transcribe(
                                    &staging, p.clone(), &cleaner, strength, mode, wav,
                                    &injector, command_countdown, &command_gen, lexicon.clone(),
                                ),
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

/// 批量后端：整段 WAV 异步转写
#[allow(clippy::too_many_arguments)]
fn spawn_transcribe(
    staging: &Staging,
    provider: Arc<dyn asr::AsrProvider>,
    cleaner: &Option<Arc<dyn TextCleaner>>,
    strength: llm::Strength,
    mode: RecordMode,
    wav: Vec<u8>,
    injector: &Arc<dyn Injector>,
    command_countdown: Duration,
    command_gen: &Arc<AtomicU64>,
    lexicon: Arc<command::Lexicon>,
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
                    clean_and_append(&staging, &cleaner, text.trim(), strength)
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
    strength: llm::Strength,
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
    tauri::async_runtime::spawn(async move {
        let result = cleaner.clean(&raw, strength).await;
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
        Some(c) => c,
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
