//! 设置事件处理（M5）。
//!
//! 注册所有设置相关的事件监听器。提示词管理已迁移至 `prompts` 模块。

use tauri::{AppHandle, Emitter, Listener, Manager};

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use crate::asr::{self, AsrBackend};
use crate::config::{self, CommandConfig, Config, MouseHotkeyConfig};
use crate::llm;
use crate::prompts::{self, PromptConfig};
use crate::hotkey::{self, Bindings, KeySpec, MouseButton};

// ── 通用配置载荷工具 ──────────────────────────────────────────

fn opt_str(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn apply_asr_payload(cfg: &mut Config, v: &serde_json::Value) {
    cfg.asr.provider = opt_str(v, "provider").unwrap_or_else(|| "bailian".to_string());
    cfg.asr.protocol = opt_str(v, "protocol");
    cfg.asr.model = opt_str(v, "model");
    cfg.asr.base_url = opt_str(v, "base_url");
    cfg.asr.api_key = opt_str(v, "api_key");
}

fn apply_llm_payload(cfg: &mut Config, v: &serde_json::Value) {
    cfg.llm.provider = opt_str(v, "provider");
    cfg.llm.protocol = opt_str(v, "protocol");
    cfg.llm.model = opt_str(v, "model");
    cfg.llm.base_url = opt_str(v, "base_url");
    cfg.llm.api_key = opt_str(v, "api_key");
    cfg.llm.strength = opt_str(v, "strength");
}

fn config_path() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".drop-typing.toml")
}

/// ASR 连通性测试：批量后端用 1 秒静音 WAV 走一遍 transcribe，
/// 实时后端建立会话并发送 100ms 静音 PCM 后 finish。
async fn run_asr_test(backend: AsrBackend) -> Result<String, String> {
    match backend {
        AsrBackend::Batch(p) => {
            let wav = asr::make_silence_wav(1);
            match tokio::time::timeout(Duration::from_secs(10), p.transcribe(&wav, None)).await {
                Ok(Ok(text)) => Ok(format!("连接成功，ASR 返回：{text}")),
                Ok(Err(e)) => Err(format!("ASR 调用失败：{e:#}")),
                Err(_) => Err("ASR 测试超时（10 秒）".to_string()),
            }
        }
        AsrBackend::Realtime(p) => {
            let result = tauri::async_runtime::spawn_blocking(move || {
                let (ptx, _prx) = mpsc::channel::<String>();
                let session = p
                    .start_session(ptx)
                    .map_err(|e| format!("会话建立失败：{e:#}"))?;
                // 100ms @16kHz/16bit/mono = 3200 字节
                session
                    .send_audio(&vec![0u8; 3200])
                    .map_err(|e| format!("发送音频失败：{e:#}"))?;
                session
                    .finish()
                    .map_err(|e| format!("识别调用失败：{e:#}"))
            })
            .await;
            match result {
                Ok(Ok(text)) => Ok(format!("连接成功，ASR 返回：{text}")),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(format!("测试线程异常：{e}")),
            }
        }
    }
}

// ── 事件处理注册 ──────────────────────────────────────────

pub fn register_settings_handlers(app: &AppHandle) {
    let app_handle = app.clone();

    // ── 试音状态（定制硬件面板）：单实例保护 + 停止信号 ──
    let testing_flag: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));
    let stop_flag: Arc<AtomicBool> = Arc::new(AtomicBool::new(false));

    // ── 暂存条请求打开设置窗口
    let stop_on_close = stop_flag.clone();
    app.listen("drop-typing://open-settings", move |_| {
        if let Some(win) = app_handle.get_webview_window("settings") {
            let _ = win.show();
            let _ = win.set_focus();
        } else {
            let win = tauri::WebviewWindowBuilder::new(
                &app_handle,
                "settings",
                tauri::WebviewUrl::App("settings.html".into()),
            )
            .title("drop-typing 设置")
            .inner_size(1100.0, 680.0)
            .resizable(true)
            .decorations(true)
            .center()
            .build();
            // 关闭设置窗口时兜底停止试音（防前端异常退出导致试音流悬挂）
            if let Ok(win) = win {
                let stop_on_close = stop_on_close.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { .. } = event {
                        stop_on_close.store(true, Ordering::SeqCst);
                    }
                });
            }
        }
    });

    // ── 暂存条主动请求样式列表
    let ah = app.clone();
    app.listen("drop-typing://get-styles", move |_| {
        emit_styles_inner(&ah);
    });

    // ── 设置页面加载完成 → 发送当前提示词 + 默认值
    let ah = app.clone();
    app.listen("drop-typing://settings-ready", move |_| {
        eprintln!("[drop-typing] settings-ready received");
        let pc = prompts::load_prompts();
        let defaults = prompts::default_prompt_config();
        let _ = ah.emit(
            "drop-typing://config",
            serde_json::json!({ "prompts": pc, "defaults": defaults }),
        );
    });

    // ── 保存提示词
    let ah = app.clone();
    app.listen("drop-typing://save-config", move |ev| {
        let raw = ev.payload();
        eprintln!("[drop-typing] save-config received, payload len={}", raw.len());
        let payload: serde_json::Value =
            serde_json::from_str(raw).unwrap_or_default();
        if let Some(prompts_json) = payload.get("prompts") {
            match serde_json::from_value::<PromptConfig>(prompts_json.clone()) {
                Ok(pc) => {
                    match prompts::save_prompts(&pc) {
                        Ok(()) => {
                            eprintln!("[drop-typing] prompts saved to ~/.drop-typing/prompts.json");
                            let _ = ah.emit(
                                "drop-typing://config-saved",
                                serde_json::json!({ "success": true }),
                            );
                        }
                        Err(e) => {
                            eprintln!("[drop-typing] prompts save failed: {e}");
                            let _ = ah.emit(
                                "drop-typing://config-saved",
                                serde_json::json!({ "success": false, "error": e }),
                            );
                        }
                    }
                }
                Err(e) => {
                    let msg = format!("提示词 JSON 解析失败：{e}");
                    eprintln!("[drop-typing] {msg}");
                    let _ = ah.emit(
                        "drop-typing://config-saved",
                        serde_json::json!({ "success": false, "error": msg }),
                    );
                }
            }
        } else {
            eprintln!("[drop-typing] save-config: missing 'prompts' field");
        }
    });

    // ── 重置提示词（移除用户配置 → 回退默认值）
    let ah = app.clone();
    app.listen("drop-typing://reset-prompt", move |ev| {
        eprintln!("[drop-typing] === reset-prompt received ===");
        eprintln!("[drop-typing] reset-prompt raw payload: {}", ev.payload());
        let payload: serde_json::Value =
            serde_json::from_str(ev.payload()).unwrap_or_default();
        let key = payload.get("key").and_then(|v| v.as_str()).unwrap_or("");
        eprintln!("[drop-typing] reset-prompt key={key}");

        let default_text = if key == "base" {
            prompts::default_base_prompt().to_string()
        } else {
            prompts::default_style_prompt(key).unwrap_or("").to_string()
        };
        eprintln!("[drop-typing] reset-prompt default_text len={}", default_text.len());

        // 从 prompts.json 移除该字段
        let mut pc = prompts::load_prompts();
        if key == "base" {
            pc.base = None;
            eprintln!("[drop-typing] reset-prompt set pc.base=None");
        } else if let Some(ref mut styles) = pc.styles {
            styles.remove(key);
            eprintln!("[drop-typing] reset-prompt removed key={key} from styles");
        }
        // 如果重置的是当前选中的样式，清除 current_style
        let (mut cfg, _) = Config::load_lenient();
        if cfg.llm.current_style.as_deref() == Some(key) {
            cfg.llm.current_style = None;
            let _ = cfg.save();
            eprintln!("[drop-typing] reset-prompt cleared current_style for key={key}");
        }
        match prompts::save_prompts(&pc) {
            Ok(()) => eprintln!("[drop-typing] reset-prompt saved to prompts.json"),
            Err(e) => eprintln!("[drop-typing] reset-prompt save failed: {e}"),
        }

        eprintln!("[drop-typing] reset-prompt emitting response, default_text len={}", default_text.len());
        let _ = ah.emit(
            "drop-typing://prompt-reset",
            serde_json::json!({ "key": key, "default_text": default_text }),
        );
        eprintln!("[drop-typing] reset-prompt response emitted");
    });

    // ── AI 优化提示词
    let ah = app.clone();
    app.listen("drop-typing://ai-optimize", move |ev| {
        eprintln!("[drop-typing] ai-optimize received");
        let payload: serde_json::Value =
            serde_json::from_str(ev.payload()).unwrap_or_default();
        let key = payload
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let text = payload
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let intent = payload
            .get("intent")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if text.is_empty() {
            let _ = ah.emit(
                "drop-typing://ai-optimize-result",
                serde_json::json!({ "key": key, "error": "提示词为空，无法优化" }),
            );
            return;
        }

        let (cfg, _) = Config::load_lenient();
        let cleaner = match crate::llm::cleaner_from_config(&cfg) {
            Some(c) => c,
            None => {
                let _ = ah.emit(
                    "drop-typing://ai-optimize-result",
                    serde_json::json!({ "key": key, "error": "请先在配置文件中配置 [llm] 段以使用 AI 优化功能" }),
                );
                return;
            }
        };

        let ah2 = ah.clone();
        tauri::async_runtime::spawn(async move {
            eprintln!("[drop-typing] ai-optimize calling LLM...");
            match ai_optimize_prompt(&*cleaner, &text, &intent).await {
                Ok(optimized) => {
                    eprintln!("[drop-typing] ai-optimize success, len={}", optimized.len());
                    let _ = ah2.emit(
                        "drop-typing://ai-optimize-result",
                        serde_json::json!({ "key": key, "optimized": optimized }),
                    );
                }
                Err(e) => {
                    eprintln!("[drop-typing] ai-optimize failed: {e}");
                    let _ = ah2.emit(
                        "drop-typing://ai-optimize-result",
                        serde_json::json!({ "key": key, "error": e }),
                    );
                }
            }
        });
    });

    // ── 新增自定义样式
    let ah = app.clone();
    app.listen("drop-typing://add-style", move |ev| {
        let payload: serde_json::Value =
            serde_json::from_str(ev.payload()).unwrap_or_default();
        let key = payload
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        // 校验
        if key.trim().is_empty() {
            let _ = ah.emit(
                "drop-typing://style-added",
                serde_json::json!({ "success": false, "error": "样式名称不能为空" }),
            );
            return;
        }
        if prompts::is_builtin_style(&key) {
            let _ = ah.emit(
                "drop-typing://style-added",
                serde_json::json!({ "success": false, "error": "不能使用与内置样式相同的名称" }),
            );
            return;
        }

        let mut pc = prompts::load_prompts();
        let styles = pc.styles.get_or_insert_with(prompts::StylePrompts::new);
        if styles.contains_key(&key) {
            let _ = ah.emit(
                "drop-typing://style-added",
                serde_json::json!({ "success": false, "error": "样式名称已存在" }),
            );
            return;
        }

        styles.insert(key.clone(), String::new());
        let _ = prompts::save_prompts(&pc);
        emit_styles_inner(&ah);
        let _ = ah.emit(
            "drop-typing://style-added",
            serde_json::json!({ "success": true, "key": key }),
        );
    });

    // ── 删除自定义样式
    let ah = app.clone();
    app.listen("drop-typing://delete-style", move |ev| {
        let payload: serde_json::Value =
            serde_json::from_str(ev.payload()).unwrap_or_default();
        let key = payload
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if prompts::is_builtin_style(&key) {
            let _ = ah.emit(
                "drop-typing://style-deleted",
                serde_json::json!({ "success": false, "error": "不能删除内置样式", "key": key }),
            );
            return;
        }

        let mut pc = prompts::load_prompts();
        let existed = pc
            .styles
            .as_mut()
            .map(|s| s.remove(&key).is_some())
            .unwrap_or(false);
        let _ = prompts::save_prompts(&pc);

        // 如果删除的是当前选中样式，清除
        let (mut cfg, _) = Config::load_lenient();
        if cfg.llm.current_style.as_deref() == Some(&key) {
            cfg.llm.current_style = None;
            let _ = cfg.save();
        }

        emit_styles_inner(&ah);
        let _ = ah.emit(
            "drop-typing://style-deleted",
            serde_json::json!({ "success": true, "key": key, "existed": existed }),
        );
    });

    // ── 唤醒词配置：获取当前配置
    let ah = app.clone();
    app.listen("drop-typing://get-wakeword-config", move |_| {
        eprintln!("[drop-typing] get-wakeword-config received");
        let (cfg, _) = Config::load_lenient();
        let defaults: Vec<serde_json::Value> = vec![
            serde_json::json!({ "keyword": "小易记", "action": "input" }),
            serde_json::json!({ "keyword": "小易修", "action": "repair" }),
            serde_json::json!({ "keyword": "小易控", "action": "command" }),
            serde_json::json!({ "keyword": "小易确认", "action": "commit" }),
            serde_json::json!({ "keyword": "小易清空", "action": "clear" }),
        ];
        let keywords: Vec<serde_json::Value> = cfg
            .wakeword
            .keywords
            .iter()
            .map(|e| {
                serde_json::json!({
                    "keyword": e.keyword,
                    "action": e.action,
                })
            })
            .collect();
        let _ = ah.emit(
            "drop-typing://wakeword-config",
            serde_json::json!({
                "keywords": keywords,
                "defaults": defaults,
                "enabled": cfg.wakeword.enabled,
                "has_custom": !cfg.wakeword.keywords.is_empty(),
                "advanced": {
                    "model_dir": &cfg.wakeword.model_dir,
                    "keywords_threshold": cfg.wakeword.keywords_threshold,
                    "keywords_score": cfg.wakeword.keywords_score,
                    "silence_timeout_ms": cfg.wakeword.silence_timeout_ms,
                    "pre_roll_ms": cfg.wakeword.pre_roll_ms,
                    "ring_buffer_duration_ms": cfg.wakeword.ring_buffer_duration_ms,
                },
            }),
        );
    });

    // ── 唤醒词配置：保存
    let ah = app.clone();
    app.listen("drop-typing://save-wakeword-config", move |ev| {
        eprintln!("[drop-typing] save-wakeword-config received");
        let payload: serde_json::Value =
            serde_json::from_str(ev.payload()).unwrap_or_default();

        let keywords: Vec<crate::config::KeywordEntry> = payload
            .get("keywords")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|item| crate::config::KeywordEntry {
                        keyword: item["keyword"].as_str().unwrap_or("").to_string(),
                        action: item["action"].as_str().unwrap_or("input").to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let (mut cfg, _) = Config::load_lenient();
        cfg.wakeword.keywords = keywords;
        cfg.wakeword.enabled = payload
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(cfg.wakeword.enabled);

        if let Some(adv) = payload.get("advanced") {
            if let Some(v) = adv.get("model_dir").and_then(|v| v.as_str()) {
                let v = v.trim();
                if !v.is_empty() {
                    cfg.wakeword.model_dir = v.to_string();
                }
            }
            if let Some(v) = adv.get("keywords_threshold").and_then(|v| v.as_f64()) {
                cfg.wakeword.keywords_threshold = v as f32;
            }
            if let Some(v) = adv.get("keywords_score").and_then(|v| v.as_f64()) {
                cfg.wakeword.keywords_score = v as f32;
            }
            if let Some(v) = adv.get("silence_timeout_ms").and_then(|v| v.as_u64()) {
                cfg.wakeword.silence_timeout_ms = v;
            }
            if let Some(v) = adv.get("pre_roll_ms").and_then(|v| v.as_u64()) {
                cfg.wakeword.pre_roll_ms = v;
            }
            if let Some(v) = adv.get("ring_buffer_duration_ms").and_then(|v| v.as_u64()) {
                cfg.wakeword.ring_buffer_duration_ms = v;
            }
        }

        match cfg.save() {
            Ok(()) => {
                eprintln!(
                    "[drop-typing] wakeword config saved ({}) keywords",
                    cfg.wakeword.keywords.len(),
                );
                let _ = ah.emit(
                    "drop-typing://wakeword-saved",
                    serde_json::json!({ "success": true }),
                );
                // 唤醒词已支持热切换：通知 pipeline 重建/停止麦克风监听
                let _ = ah.emit("drop-typing://runtime-reload", serde_json::json!({}));
            }
            Err(e) => {
                eprintln!("[drop-typing] wakeword config save failed: {e}");
                let _ = ah.emit(
                    "drop-typing://wakeword-saved",
                    serde_json::json!({ "success": false, "error": e.to_string() }),
                );
            }
        }
    });

    // ── 唤醒词配置：重置为默认值
    let ah = app.clone();
    app.listen("drop-typing://reset-wakeword-config", move |_| {
        eprintln!("[drop-typing] reset-wakeword-config received");
        let (mut cfg, _) = Config::load_lenient();
        cfg.wakeword.keywords = Vec::new();
        match cfg.save() {
            Ok(()) => {
                eprintln!("[drop-typing] wakeword config reset to defaults");
                let _ = ah.emit(
                    "drop-typing://wakeword-reset",
                    serde_json::json!({ "success": true }),
                );
                let _ = ah.emit("drop-typing://runtime-reload", serde_json::json!({}));
            }
            Err(e) => {
                let _ = ah.emit(
                    "drop-typing://wakeword-reset",
                    serde_json::json!({ "success": false, "error": e.to_string() }),
                );
            }
        }
    });

    // ── 唤醒词 token 预览（调用 text2token 动态计算）
    let ah = app.clone();
    app.listen("drop-typing://preview-wakeword-tokens", move |ev| {
        eprintln!("[drop-typing] preview-wakeword-tokens received");
        let payload: serde_json::Value =
            serde_json::from_str(ev.payload()).unwrap_or_default();

        let keywords: Vec<(String, String)> = payload
            .get("keywords")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|item| {
                        (
                            item["keyword"].as_str().unwrap_or("").to_string(),
                            item["keyword"].as_str().unwrap_or("").to_string(),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        if keywords.is_empty() {
            let _ = ah.emit(
                "drop-typing://wakeword-tokens",
                serde_json::json!({ "error": "没有关键词" }),
            );
            return;
        }

        // 查找模型目录
        let (cfg, _) = Config::load_lenient();
        let model_dir = match crate::wakeword::sherpa::resolve_model_dir(
            &cfg.wakeword.model_dir,
            &std::path::PathBuf::new(),
        ) {
            Some(d) => d,
            None => {
                let _ = ah.emit(
                    "drop-typing://wakeword-tokens",
                    serde_json::json!({ "error": "模型目录未找到" }),
                );
                return;
            }
        };

        let t2t = match crate::wakeword::text2token::Text2Token::load(&model_dir) {
            Ok(t) => t,
            Err(e) => {
                let _ = ah.emit(
                    "drop-typing://wakeword-tokens",
                    serde_json::json!({ "error": format!("text2token 加载失败：{e}") }),
                );
                return;
            }
        };

        match t2t.convert_batch(&keywords) {
            Ok(lines) => {
                let _ = ah.emit(
                    "drop-typing://wakeword-tokens",
                    serde_json::json!({ "lines": lines }),
                );
            }
            Err(e) => {
                let _ = ah.emit(
                    "drop-typing://wakeword-tokens",
                    serde_json::json!({ "error": format!("转换失败：{e}") }),
                );
            }
        }
    });

    // ── 定制硬件（音频设备）：获取设备列表与当前选择
    let ah = app.clone();
    app.listen("drop-typing://get-audio-config", move |_| {
        eprintln!("[drop-typing] get-audio-config received");
        let (cfg, _) = Config::load_lenient();
        let (all, default_name) = {
            let all = crate::audio::list_input_devices().unwrap_or_default();
            let default_name = all
                .iter()
                .find(|(_, is_default)| *is_default)
                .map(|(name, _)| name.clone());
            (all, default_name)
        };
        let devices: Vec<serde_json::Value> = all
            .into_iter()
            .map(|(name, is_default)| {
                serde_json::json!({ "name": name, "is_default": is_default })
            })
            .collect();
        let _ = ah.emit(
            "drop-typing://audio-config",
            serde_json::json!({
                "devices": devices,
                "current": cfg.audio.input_device,
                "default_name": default_name,
            }),
        );
    });

    // ── 定制硬件（音频设备）：保存设备选择 → 写盘 + 热切换
    let ah = app.clone();
    app.listen("drop-typing://save-audio-config", move |ev| {
        eprintln!("[drop-typing] save-audio-config received");
        let payload: serde_json::Value =
            serde_json::from_str(ev.payload()).unwrap_or_default();
        let input_device = payload
            .get("input_device")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let (mut cfg, _) = Config::load_lenient();
        cfg.audio.input_device = input_device;
        match cfg.save() {
            Ok(()) => {
                eprintln!("[drop-typing] audio config saved");
                let _ = ah.emit(
                    "drop-typing://audio-config-saved",
                    serde_json::json!({ "success": true }),
                );
                // 通知 pipeline 热切换录音器与唤醒词监听（无需重启）
                let _ = ah.emit("drop-typing://runtime-reload", serde_json::json!({}));
            }
            Err(e) => {
                eprintln!("[drop-typing] audio config save failed: {e}");
                let _ = ah.emit(
                    "drop-typing://audio-config-saved",
                    serde_json::json!({ "success": false, "error": e.to_string() }),
                );
            }
        }
    });

    // ── 试音：开始（单实例保护，重复开始忽略）
    let testing_start = testing_flag.clone();
    let stop_start = stop_flag.clone();
    let app_for_test = app.clone();
    app.listen("drop-typing://start-sound-test", move |_| {
        if testing_start.swap(true, Ordering::SeqCst) {
            return; // 已在试音中
        }
        stop_start.store(false, Ordering::SeqCst);
        let app_for_test = app_for_test.clone();
        let stop_done = stop_start.clone();
        let testing_done = testing_start.clone();
        std::thread::spawn(move || {
            if let Err(e) =
                crate::audio::run_sound_level_meter(app_for_test.clone(), stop_done.clone())
            {
                eprintln!("[drop-typing] 试音启动失败：{e}");
                let _ = app_for_test.emit(
                    "drop-typing://sound-test-error",
                    serde_json::json!({ "message": e.to_string() }),
                );
            }
            stop_done.store(false, Ordering::SeqCst);
            testing_done.store(false, Ordering::SeqCst);
        });
    });

    // ── 试音：停止
    let stop_stop = stop_flag.clone();
    app.listen("drop-typing://stop-sound-test", move |_| {
        stop_stop.store(true, Ordering::SeqCst);
    });

    // ── 快捷键配置：获取当前配置
    let ah = app.clone();
    app.listen("drop-typing://get-shortcut-config", move |_| {
        eprintln!("[drop-typing] get-shortcut-config received");
        let (cfg, _) = Config::load_lenient();

        let current = cfg.hotkey_bindings();
        let defaults = Bindings::platform_default();

        let specs_to_strings = |s: &[KeySpec]| -> Vec<String> {
            s.iter().map(|ks| ks.to_config_name()).collect()
        };

        let mouse_to_name = |b: Option<MouseButton>| -> Option<&str> {
            b.map(|mb| match mb {
                MouseButton::Forward => "forward",
                MouseButton::Back => "back",
            })
        };

        let has_custom = cfg.hotkey.keyboard.trigger.is_some()
            || cfg.hotkey.keyboard.repair.is_some()
            || cfg.hotkey.keyboard.command.is_some()
            || cfg.hotkey.keyboard.cancel.is_some()
            || cfg.hotkey.mouse.is_some();

        let _ = ah.emit(
            "drop-typing://shortcut-config",
            serde_json::json!({
                "platform": hotkey::platform_name(),
                "keyboard": {
                    "trigger": specs_to_strings(&current.trigger),
                    "repair":  specs_to_strings(&current.repair),
                    "command": specs_to_strings(&current.command),
                    "cancel":  specs_to_strings(&current.cancel),
                },
                "mouse": {
                    "trigger": mouse_to_name(current.mouse.trigger),
                    "repair":  mouse_to_name(current.mouse.repair),
                },
                "defaults": {
                    "keyboard": {
                        "trigger": specs_to_strings(&defaults.trigger),
                        "repair":  specs_to_strings(&defaults.repair),
                        "command": specs_to_strings(&defaults.command),
                        "cancel":  specs_to_strings(&defaults.cancel),
                    },
                    "mouse": {
                        "trigger": mouse_to_name(defaults.mouse.trigger),
                        "repair":  mouse_to_name(defaults.mouse.repair),
                    },
                },
                "has_custom": has_custom,
            }),
        );
    });

    // ── 快捷键配置：保存
    let ah = app.clone();
    app.listen("drop-typing://save-shortcut-config", move |ev| {
        eprintln!("[drop-typing] save-shortcut-config received");
        let payload: serde_json::Value =
            serde_json::from_str(ev.payload()).unwrap_or_default();

        let (mut cfg, _) = Config::load_lenient();

        // 解析键盘段
        if let Some(kb) = payload.get("keyboard") {
            let parse_channel = |name: &str| -> Option<Vec<String>> {
                kb.get(name)
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .filter(|s| !s.is_empty())
                            .collect()
                    })
            };
            cfg.hotkey.keyboard.trigger = parse_channel("trigger");
            cfg.hotkey.keyboard.repair  = parse_channel("repair");
            cfg.hotkey.keyboard.command = parse_channel("command");
            cfg.hotkey.keyboard.cancel  = parse_channel("cancel");
        }

        // 解析鼠标段
        if let Some(mouse) = payload.get("mouse") {
            let parse_mouse_button = |name: &str| -> Option<crate::config::MouseButton> {
                mouse.get(name).and_then(|v| v.as_str()).and_then(|s| match s {
                    "forward" => Some(crate::config::MouseButton::Forward),
                    "back" => Some(crate::config::MouseButton::Back),
                    _ => None,
                })
            };
            let trigger = parse_mouse_button("trigger");
            let repair  = parse_mouse_button("repair");
            if trigger.is_some() || repair.is_some() {
                cfg.hotkey.mouse = Some(MouseHotkeyConfig { trigger, repair });
            } else {
                cfg.hotkey.mouse = None;
            }
        }

        match cfg.save() {
            Ok(()) => {
                eprintln!("[drop-typing] shortcut config saved");
                let _ = ah.emit(
                    "drop-typing://shortcut-saved",
                    serde_json::json!({ "success": true }),
                );
            }
            Err(e) => {
                eprintln!("[drop-typing] shortcut config save failed: {e}");
                let _ = ah.emit(
                    "drop-typing://shortcut-saved",
                    serde_json::json!({ "success": false, "error": e.to_string() }),
                );
            }
        }
    });

    // ── 快捷键配置：重置为平台默认值
    let ah = app.clone();
    app.listen("drop-typing://reset-shortcut-config", move |_| {
        eprintln!("[drop-typing] reset-shortcut-config received");
        let (mut cfg, _) = Config::load_lenient();
        cfg.hotkey = crate::config::HotkeyConfig::default();
        match cfg.save() {
            Ok(()) => {
                eprintln!("[drop-typing] shortcut config reset to defaults");
                let _ = ah.emit(
                    "drop-typing://shortcut-reset",
                    serde_json::json!({ "success": true }),
                );
            }
            Err(e) => {
                let _ = ah.emit(
                    "drop-typing://shortcut-reset",
                    serde_json::json!({ "success": false, "error": e.to_string() }),
                );
            }
        }
    });

    // ── 通用配置（模型 / 毫秒）：获取
    let ah = app.clone();
    app.listen("drop-typing://get-general-config", move |_| {
        eprintln!("[drop-typing] get-general-config received");
        let (cfg, _) = Config::load_lenient();
        let _ = ah.emit(
            "drop-typing://general-config",
            serde_json::json!({
                "asr": {
                    "provider": &cfg.asr.provider,
                    "protocol": &cfg.asr.protocol,
                    "model": &cfg.asr.model,
                    "base_url": &cfg.asr.base_url,
                    "api_key": &cfg.asr.api_key,
                },
                "llm": {
                    "provider": &cfg.llm.provider,
                    "protocol": &cfg.llm.protocol,
                    "model": &cfg.llm.model,
                    "base_url": &cfg.llm.base_url,
                    "api_key": &cfg.llm.api_key,
                    "strength": &cfg.llm.strength,
                },
                "thresholds": {
                    "long_press_threshold_ms": cfg.long_press_threshold_ms,
                    "double_press_window_ms": cfg.double_press_window_ms,
                    "command_countdown_ms": cfg.command_countdown_ms,
                },
                "effective_command_countdown_ms": cfg.effective_command_countdown_ms(),
            }),
        );
    });

    // ── 通用配置（模型 / 毫秒）：保存
    let ah = app.clone();
    app.listen("drop-typing://save-general-config", move |ev| {
        eprintln!("[drop-typing] save-general-config received");
        let payload: serde_json::Value =
            serde_json::from_str(ev.payload()).unwrap_or_default();
        let mut cfg = Config::load_lenient().0;

        if let Some(asr) = payload.get("asr") {
            apply_asr_payload(&mut cfg, asr);
        }
        if let Some(llm) = payload.get("llm") {
            apply_llm_payload(&mut cfg, llm);
        }
        if let Some(th) = payload.get("thresholds") {
            for (key, slot) in [
                ("long_press_threshold_ms", &mut cfg.long_press_threshold_ms),
                ("double_press_window_ms", &mut cfg.double_press_window_ms),
                ("command_countdown_ms", &mut cfg.command_countdown_ms),
            ] {
                match th.get(key).and_then(|v| v.as_u64()) {
                    Some(n) if (50..=10_000).contains(&n) => *slot = n,
                    Some(n) => {
                        let _ = ah.emit(
                            "drop-typing://config-saved",
                            serde_json::json!({
                                "success": false,
                                "error": format!("{key} 超出范围（50~10000ms）：{n}"),
                            }),
                        );
                        return;
                    }
                    None => {
                        let _ = ah.emit(
                            "drop-typing://config-saved",
                            serde_json::json!({
                                "success": false,
                                "error": format!("{key} 缺失或格式错误"),
                            }),
                        );
                        return;
                    }
                }
            }
        }

        match cfg.save() {
            Ok(()) => {
                let _ = ah.emit(
                    "drop-typing://config-saved",
                    serde_json::json!({ "success": true }),
                );
                let _ = ah.emit("drop-typing://runtime-reload", serde_json::json!({}));
            }
            Err(e) => {
                eprintln!("[drop-typing] general config save failed: {e}");
                let _ = ah.emit(
                    "drop-typing://config-saved",
                    serde_json::json!({ "success": false, "error": e }),
                );
            }
        }
    });

    // ── 语音控制（Command 词表 + 倒计时）：获取
    let ah = app.clone();
    app.listen("drop-typing://get-command-config", move |_| {
        eprintln!("[drop-typing] get-command-config received");
        let (cfg, _) = Config::load_lenient();
        let cmd = serde_json::to_value(&cfg.command).unwrap_or_default();
        let _ = ah.emit(
            "drop-typing://command-config",
            serde_json::json!({
                "config": cmd,
                "effective_command_countdown_ms": cfg.effective_command_countdown_ms(),
            }),
        );
    });

    // ── 语音控制（Command 词表 + 倒计时）：保存
    let ah = app.clone();
    app.listen("drop-typing://save-command-config", move |ev| {
        eprintln!("[drop-typing] save-command-config received");
        let payload: serde_json::Value =
            serde_json::from_str(ev.payload()).unwrap_or_default();
        match serde_json::from_value::<CommandConfig>(payload) {
            Ok(cmd) => {
                let mut cfg = Config::load_lenient().0;
                cfg.command = cmd;
                match cfg.save() {
                    Ok(()) => {
                        let _ = ah.emit(
                            "drop-typing://command-config-saved",
                            serde_json::json!({ "success": true }),
                        );
                        let _ = ah.emit("drop-typing://runtime-reload", serde_json::json!({}));
                    }
                    Err(e) => {
                        eprintln!("[drop-typing] command config save failed: {e}");
                        let _ = ah.emit(
                            "drop-typing://command-config-saved",
                            serde_json::json!({ "success": false, "error": e }),
                        );
                    }
                }
            }
            Err(e) => {
                let msg = format!("指令配置解析失败：{e}");
                eprintln!("[drop-typing] {msg}");
                let _ = ah.emit(
                    "drop-typing://command-config-saved",
                    serde_json::json!({ "success": false, "error": msg }),
                );
            }
        }
    });

    // ── 配置文件兜底编辑器：获取原文
    let ah = app.clone();
    app.listen("drop-typing://get-config-file", move |_| {
        eprintln!("[drop-typing] get-config-file received");
        let path = config_path();
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let _ = ah.emit(
            "drop-typing://config-file",
            serde_json::json!({
                "text": text,
                "path": path.display().to_string(),
                "exists": path.exists(),
            }),
        );
    });

    // ── 配置文件兜底编辑器：校验 + 整文件写回
    let ah = app.clone();
    app.listen("drop-typing://save-config-file", move |ev| {
        eprintln!("[drop-typing] save-config-file received");
        let payload: serde_json::Value =
            serde_json::from_str(ev.payload()).unwrap_or_default();
        let text = payload
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        match config::parse_config_file(&text) {
            Err(e) => {
                let _ = ah.emit(
                    "drop-typing://config-file-saved",
                    serde_json::json!({ "success": false, "error": e }),
                );
            }
            Ok(new_cfg) => {
                let old_cfg = Config::load_lenient().0;
                let path = config_path();
                let write_result = (|| -> Result<(), String> {
                    if let Some(parent) = path.parent() {
                        std::fs::create_dir_all(parent)
                            .map_err(|e| format!("无法创建配置目录 {}：{e}", parent.display()))?;
                    }
                    std::fs::write(&path, text.as_bytes())
                        .map_err(|e| format!("配置文件写入失败（{}）：{e}", path.display()))
                })();

                match write_result {
                    Ok(()) => {
                        let restart = config::needs_restart(&old_cfg, &new_cfg);
                        let _ = ah.emit("drop-typing://runtime-reload", serde_json::json!({}));
                        let _ = ah.emit(
                            "drop-typing://config-file-saved",
                            serde_json::json!({
                                "success": true,
                                "restart_required": restart,
                            }),
                        );
                        if restart {
                            let _ = ah.emit(
                                "drop-typing://restart-required",
                                serde_json::json!({
                                    "message": "配置中热键设置已变化，需要重启应用后才能生效。",
                                }),
                            );
                        }
                    }
                    Err(e) => {
                        let _ = ah.emit(
                            "drop-typing://config-file-saved",
                            serde_json::json!({ "success": false, "error": e }),
                        );
                    }
                }
            }
        }
    });

    // ── ASR 测试连接
    let ah = app.clone();
    app.listen("drop-typing://test-asr", move |ev| {
        eprintln!("[drop-typing] test-asr received");
        let payload: serde_json::Value =
            serde_json::from_str(ev.payload()).unwrap_or_default();
        let mut cfg = Config::load_lenient().0;
        if let Some(asr) = payload.get("asr") {
            apply_asr_payload(&mut cfg, asr);
        }
        let Some(backend) = asr::backend_from_config(&cfg) else {
            let _ = ah.emit(
                "drop-typing://test-asr-result",
                serde_json::json!({
                    "success": false,
                    "message": "配置无效：缺少 API Key 或协议未知",
                }),
            );
            return;
        };

        let ah2 = ah.clone();
        tauri::async_runtime::spawn(async move {
            let result = run_asr_test(backend).await;
            let (success, message) = match result {
                Ok(msg) => (true, msg),
                Err(e) => (false, e),
            };
            let _ = ah2.emit(
                "drop-typing://test-asr-result",
                serde_json::json!({ "success": success, "message": message }),
            );
        });
    });

    // ── LLM 测试连接
    let ah = app.clone();
    app.listen("drop-typing://test-llm", move |ev| {
        eprintln!("[drop-typing] test-llm received");
        let payload: serde_json::Value =
            serde_json::from_str(ev.payload()).unwrap_or_default();
        let mut cfg = Config::load_lenient().0;
        if let Some(llm) = payload.get("llm") {
            apply_llm_payload(&mut cfg, llm);
        }
        let Some(cleaner) = llm::cleaner_from_config(&cfg) else {
            let _ = ah.emit(
                "drop-typing://test-llm-result",
                serde_json::json!({
                    "success": false,
                    "message": "配置无效：缺少 LLM API Key 或协议未知",
                }),
            );
            return;
        };

        let ah2 = ah.clone();
        tauri::async_runtime::spawn(async move {
            let result =
                tokio::time::timeout(Duration::from_secs(15), cleaner.clean("你好", "")).await;
            let (success, message) = match &result {
                Ok(Ok(text)) => (
                    true,
                    format!(
                        "连接成功，LLM 返回：{}",
                        text.trim().chars().take(60).collect::<String>()
                    ),
                ),
                Ok(Err(e)) => (false, format!("LLM 调用失败：{e:#}")),
                Err(_) => (false, "LLM 测试超时（15 秒）".to_string()),
            };
            let _ = ah2.emit(
                "drop-typing://test-llm-result",
                serde_json::json!({ "success": success, "message": message }),
            );
        });
    });

    // ── 组合键录制（动作别名 / 快捷键面板）：开始
    let ah = app.clone();
    app.listen("drop-typing://start-combo-capture", move |ev| {
        eprintln!("[drop-typing] start-combo-capture received");
        let payload: serde_json::Value =
            serde_json::from_str(ev.payload()).unwrap_or_default();
        let distinguish_sides = payload
            .get("distinguish_sides")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let mode = payload
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("combo")
            .to_string();
        let ah = ah.clone();
        std::thread::spawn(move || {
            let (success, modifiers, key, single_key, error) = if mode == "single" {
                match hotkey::capture_single(Duration::from_secs(10), distinguish_sides) {
                    Ok(k) => (true, Vec::new(), String::new(), Some(k), String::new()),
                    Err(e) => (false, Vec::new(), String::new(), None, e),
                }
            } else {
                match hotkey::capture_combo(Duration::from_secs(10), distinguish_sides) {
                    Ok(c) => (true, c.modifiers, c.key, None, String::new()),
                    Err(e) => (false, Vec::new(), String::new(), None, e),
                }
            };
            let _ = ah.emit(
                "drop-typing://combo-captured",
                serde_json::json!({
                    "success": success,
                    "modifiers": modifiers,
                    "key": key,
                    "single_key": single_key,
                    "error": error,
                }),
            );
        });
    });

    // ── 组合键录制：取消
    app.listen("drop-typing://stop-combo-capture", move |_| {
        hotkey::cancel_capture();
    });

    app.listen("drop-typing://restart", move |_| {
        eprintln!("[drop-typing] restart requested");
        let exe = std::env::current_exe().unwrap_or_default();

        // 从 executable 路径向上找到 .app 包
        let app_bundle = {
            let mut p = exe.clone();
            // exe 通常在 MyApp.app/Contents/MacOS/binary
            while p.parent().is_some() {
                if p.extension().map_or(false, |e| e == "app") {
                    break;
                }
                p = p.parent().unwrap().to_path_buf();
            }
            p
        };

        if app_bundle.extension().map_or(false, |e| e == "app") {
            eprintln!(
                "[drop-typing] 重启：open -n {}",
                app_bundle.display(),
            );
            let _ = std::process::Command::new("open")
                .args(["-n", "-a"])
                .arg(&app_bundle)
                .spawn();
        } else {
            // 回退：直接启动可执行文件
            eprintln!(
                "[drop-typing] 重启：直接启动 {}",
                exe.display(),
            );
            let _ = std::process::Command::new(&exe).spawn();
        }

        std::process::exit(0);
    });
}

// ── 启动时 / 请求时事件 ──────────────────────────────────────────

pub fn emit_styles(app: &AppHandle) {
    emit_styles_inner(app);
}

fn emit_styles_inner(app: &AppHandle) {
    let (cfg, _) = Config::load_lenient();
    let current = cfg.llm.current_style.clone();
    let pc = prompts::load_prompts();

    let mut styles: Vec<serde_json::Value> = Vec::new();
    // 内置样式始终在列表中
    for key in prompts::BUILTIN_STYLE_KEYS {
        styles.push(serde_json::json!({
            "key": key,
            "label": prompts::style_label(key),
            "builtin": true,
        }));
    }
    // 自定义样式从 prompts 中收集
    if let Some(ref sp) = pc.styles {
        for key in sp.keys() {
            if !prompts::is_builtin_style(key) {
                styles.push(serde_json::json!({
                    "key": key,
                    "label": prompts::style_label(key),
                    "builtin": false,
                }));
            }
        }
    }
    let _ = app.emit(
        "drop-typing://styles",
        serde_json::json!({ "styles": styles, "current": current }),
    );
}

// ── AI 优化 ──────────────────────────────────────────

async fn ai_optimize_prompt(
    cleaner: &dyn crate::llm::TextCleaner,
    current: &str,
    intent: &str,
) -> Result<String, String> {
    let meta_prompt = format!(
        "你是一个提示词优化专家。用户有一段用于语音转写后处理的系统提示词。\
         这段提示词会发送给大模型，指示它如何清洗和改写口语化的语音转写文本。\n\n\
         以下是用户提出的优化意图，请根据此意图改写提示词：\n\n\
         【优化意图】\n{intent}\n\n\
         【当前提示词】\n{current}\n\n\
         改写要求：\n\
         1. 保持提示词的结构化格式，使用 1. 2. 3. 这样的编号；\n\
         2. 确保指令清晰、无歧义，每条规则独立成行；\n\
         3. 保留末尾「只输出清洗后的正文，不要输出任何解释、引号或前后缀」这条关键约束；\n\
         4. 只输出优化后的提示词全文，不要输出任何解释或前后缀。"
    );

    let system_prompt =
        "你是一个提示词优化专家，擅长将模糊的自然语言需求转化为精准的结构化指令。\
         只输出优化后的提示词全文，不要输出任何额外内容。";

    let result = cleaner
        .clean(&meta_prompt, system_prompt)
        .await
        .map_err(|e| format!("LLM 调用失败：{e:#}"))?;

    if result.trim().is_empty() {
        return Err("LLM 返回了空文本".to_string());
    }
    Ok(result)
}
