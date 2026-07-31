//! 设置事件处理（M5）。
//!
//! 注册所有设置相关的事件监听器。提示词管理已迁移至 `prompts` 模块。

use tauri::{AppHandle, Emitter, Listener, Manager};

use crate::config::Config;
use crate::prompts::{self, PromptConfig};
use crate::hotkey::{self, Bindings, KeySpec, MouseButton};
use crate::config::MouseHotkeyConfig;

// ── 事件处理注册 ──────────────────────────────────────────

pub fn register_settings_handlers(app: &AppHandle) {
    let app_handle = app.clone();

    // ── 暂存条请求打开设置窗口
    app.listen("drop-typing://open-settings", move |_| {
        if let Some(win) = app_handle.get_webview_window("settings") {
            let _ = win.show();
            let _ = win.set_focus();
        } else {
            let _ = tauri::WebviewWindowBuilder::new(
                &app_handle,
                "settings",
                tauri::WebviewUrl::App("settings.html".into()),
            )
            .title("drop-typing 设置")
            .inner_size(900.0, 600.0)
            .resizable(true)
            .decorations(true)
            .center()
            .build();
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
            serde_json::json!({ "keyword": "DT打", "action": "input" }),
            serde_json::json!({ "keyword": "DT修", "action": "repair" }),
            serde_json::json!({ "keyword": "DT控", "action": "command" }),
            serde_json::json!({ "keyword": "DT确认", "action": "commit" }),
            serde_json::json!({ "keyword": "DT清空", "action": "clear" }),
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
    let ah = app.clone();
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
