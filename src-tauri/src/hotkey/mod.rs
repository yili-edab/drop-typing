//! 全局热键抽象（平台相关）。
//!
//! M1 选型说明：
//! - tauri-plugin-global-shortcut 面向"组合键按下即触发"，
//!   对"裸右 ⌘ 单独按下 + press/release 事件 + 时长判定"支持不足，故未采用。
//! - macOS 实现使用 rdev 的全局事件监听（CGEventTap），
//!   可精确拿到 RightMeta 的 press / release 事件。需要辅助功能权限。
//!
//! Windows 实现使用 rdev 低级键盘钩子（WH_KEYBOARD_LL），支持多键组合。

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

use std::sync::mpsc;

use anyhow::Result;
use rdev::Key;

/// 热键事件。时长判定（短按/长按）放在 pipeline 做，本层只报原始按下/松开。
#[derive(Debug, Clone)]
pub enum HotkeyEvent {
    /// 输入/提交通道按下（长按录音，短按提交）
    TriggerDown,
    /// 输入/提交通道松开
    TriggerUp,
    /// 修正/控制通道按下
    RepairDown,
    /// 修正/控制通道松开
    RepairUp,
    /// 指令/修复通道按下
    CommandDown,
    /// 指令/修复通道松开
    CommandUp,
    /// 录音期间有其它键按下（说明被当作组合键修饰键使用，应当作废本次录音）
    OtherKeyDown,
    /// Esc 按下：清空暂存条并隐藏
    CancelDown,
    /// 监听器运行时错误（如权限被收回）
    Error(String),
}

// ── 快捷键配置类型 ────────────────────────────────────────────────

/// 修饰键家族（左右合并），在配置中作为"Control"/"Meta"/"Alt"/"Shift"的快捷写法。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModFamily {
    Control,
    Meta,
    Alt,
    Shift,
}

impl ModFamily {
    /// 检查某个 rdev Key 是否属于此家族
    pub fn matches(&self, key: &Key) -> bool {
        match self {
            ModFamily::Control => matches!(key, Key::ControlLeft | Key::ControlRight),
            ModFamily::Meta => matches!(key, Key::MetaLeft | Key::MetaRight),
            ModFamily::Alt => matches!(key, Key::Alt | Key::AltGr),
            ModFamily::Shift => matches!(key, Key::ShiftLeft | Key::ShiftRight),
        }
    }

    /// 家族名列表（用于错误提示）
    pub fn family_names() -> &'static [&'static str] {
        &["Control", "Meta", "Alt", "Shift"]
    }
}

/// 一个按键规格：可以是整个修饰键家族（左右皆可），也可以是精确的 rdev Key。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySpec {
    /// 匹配该家族中的任意键（如 "Control" 匹配 ControlLeft 或 ControlRight）
    Family(ModFamily),
    /// 精确匹配某个 rdev Key 变体
    Exact(Key),
}

impl KeySpec {
    /// 从配置字符串解析。
    ///
    /// 支持两种写法：
    /// - 家族名（大小写不敏感）：`Control`/`Ctrl`、`Meta`/`Win`/`Cmd`、`Alt`/`Opt`、`Shift`
    /// - rdev Key 精确名：`ControlLeft`、`MetaRight`、`AltGr`、`Escape`、`Space`、
    ///   `KeyA`…`KeyZ`、`F1`…`F12` 等
    pub fn parse(s: &str) -> Result<Self, String> {
        // 先尝试家族名（大小写不敏感）
        match s.to_ascii_lowercase().as_str() {
            "control" | "ctrl" => return Ok(KeySpec::Family(ModFamily::Control)),
            "meta" | "win" | "command" | "cmd" | "super" => {
                return Ok(KeySpec::Family(ModFamily::Meta))
            }
            "alt" | "option" | "opt" => return Ok(KeySpec::Family(ModFamily::Alt)),
            "shift" => return Ok(KeySpec::Family(ModFamily::Shift)),
            _ => {}
        }

        // 再尝试精确 Key 名（大小写不敏感）
        let key = parse_key_name(s)?;
        Ok(KeySpec::Exact(key))
    }

    /// 检查某个 rdev Key 是否匹配此规格
    pub fn matches(&self, key: &Key) -> bool {
        match self {
            KeySpec::Family(f) => f.matches(key),
            KeySpec::Exact(k) => key == k,
        }
    }
}

/// 解析 rdev Key 变体名（大小写不敏感）
fn parse_key_name(s: &str) -> Result<Key, String> {
    match s.to_ascii_lowercase().as_str() {
        "alt" => Ok(Key::Alt),
        "altgr" => Ok(Key::AltGr),
        "backspace" => Ok(Key::Backspace),
        "capslock" => Ok(Key::CapsLock),
        "controlleft" | "leftcontrol" => Ok(Key::ControlLeft),
        "controlright" | "rightcontrol" => Ok(Key::ControlRight),
        "delete" => Ok(Key::Delete),
        "downarrow" | "down" => Ok(Key::DownArrow),
        "end" => Ok(Key::End),
        "escape" | "esc" => Ok(Key::Escape),
        "f1" => Ok(Key::F1),
        "f2" => Ok(Key::F2),
        "f3" => Ok(Key::F3),
        "f4" => Ok(Key::F4),
        "f5" => Ok(Key::F5),
        "f6" => Ok(Key::F6),
        "f7" => Ok(Key::F7),
        "f8" => Ok(Key::F8),
        "f9" => Ok(Key::F9),
        "f10" => Ok(Key::F10),
        "f11" => Ok(Key::F11),
        "f12" => Ok(Key::F12),
        "home" => Ok(Key::Home),
        "insert" => Ok(Key::Insert),
        "leftarrow" | "left" => Ok(Key::LeftArrow),
        "metaleft" | "leftmeta" | "winleft" | "leftwin" => Ok(Key::MetaLeft),
        "metaright" | "rightmeta" | "winright" | "rightwin" => Ok(Key::MetaRight),
        "numlock" => Ok(Key::NumLock),
        "pagedown" => Ok(Key::PageDown),
        "pageup" => Ok(Key::PageUp),
        "pause" => Ok(Key::Pause),
        "printscreen" => Ok(Key::PrintScreen),
        "return" | "enter" => Ok(Key::Return),
        "rightarrow" | "right" => Ok(Key::RightArrow),
        "scrolllock" => Ok(Key::ScrollLock),
        "shiftleft" | "leftshift" => Ok(Key::ShiftLeft),
        "shiftright" | "rightshift" => Ok(Key::ShiftRight),
        "space" => Ok(Key::Space),
        "tab" => Ok(Key::Tab),
        "uparrow" | "up" => Ok(Key::UpArrow),
        // 字母键
        "a" | "keya" => Ok(Key::KeyA),
        "b" | "keyb" => Ok(Key::KeyB),
        "c" | "keyc" => Ok(Key::KeyC),
        "d" | "keyd" => Ok(Key::KeyD),
        "e" | "keye" => Ok(Key::KeyE),
        "f" | "keyf" => Ok(Key::KeyF),
        "g" | "keyg" => Ok(Key::KeyG),
        "h" | "keyh" => Ok(Key::KeyH),
        "i" | "keyi" => Ok(Key::KeyI),
        "j" | "keyj" => Ok(Key::KeyJ),
        "k" | "keyk" => Ok(Key::KeyK),
        "l" | "keyl" => Ok(Key::KeyL),
        "m" | "keym" => Ok(Key::KeyM),
        "n" | "keyn" => Ok(Key::KeyN),
        "o" | "keyo" => Ok(Key::KeyO),
        "p" | "keyp" => Ok(Key::KeyP),
        "q" | "keyq" => Ok(Key::KeyQ),
        "r" | "keyr" => Ok(Key::KeyR),
        "s" | "keys" => Ok(Key::KeyS),
        "t" | "keyt" => Ok(Key::KeyT),
        "u" | "keyu" => Ok(Key::KeyU),
        "v" | "keyv" => Ok(Key::KeyV),
        "w" | "keyw" => Ok(Key::KeyW),
        "x" | "keyx" => Ok(Key::KeyX),
        "y" | "keyy" => Ok(Key::KeyY),
        "z" | "keyz" => Ok(Key::KeyZ),
        // 数字键
        "0" | "num0" => Ok(Key::Num0),
        "1" | "num1" => Ok(Key::Num1),
        "2" | "num2" => Ok(Key::Num2),
        "3" | "num3" => Ok(Key::Num3),
        "4" | "num4" => Ok(Key::Num4),
        "5" | "num5" => Ok(Key::Num5),
        "6" | "num6" => Ok(Key::Num6),
        "7" | "num7" => Ok(Key::Num7),
        "8" | "num8" => Ok(Key::Num8),
        "9" | "num9" => Ok(Key::Num9),
        // 符号键
        "backquote" | "`" => Ok(Key::BackQuote),
        "minus" | "-" => Ok(Key::Minus),
        "equal" | "=" => Ok(Key::Equal),
        "leftbracket" | "[" => Ok(Key::LeftBracket),
        "rightbracket" | "]" => Ok(Key::RightBracket),
        "semicolon" | ";" => Ok(Key::SemiColon),
        "quote" | "'" => Ok(Key::Quote),
        "backslash" | "\\" => Ok(Key::BackSlash),
        "intlbackslash" => Ok(Key::IntlBackslash),
        "comma" | "," => Ok(Key::Comma),
        "dot" | "." => Ok(Key::Dot),
        "slash" | "/" => Ok(Key::Slash),
        // 小键盘
        "kpreturn" | "kpenter" => Ok(Key::KpReturn),
        "kpminus" => Ok(Key::KpMinus),
        "kpplus" => Ok(Key::KpPlus),
        "kpmultiply" => Ok(Key::KpMultiply),
        "kpdivide" => Ok(Key::KpDivide),
        "kp0" => Ok(Key::Kp0),
        "kp1" => Ok(Key::Kp1),
        "kp2" => Ok(Key::Kp2),
        "kp3" => Ok(Key::Kp3),
        "kp4" => Ok(Key::Kp4),
        "kp5" => Ok(Key::Kp5),
        "kp6" => Ok(Key::Kp6),
        "kp7" => Ok(Key::Kp7),
        "kp8" => Ok(Key::Kp8),
        "kp9" => Ok(Key::Kp9),
        "kpdelete" => Ok(Key::KpDelete),
        "function" | "fn" => Ok(Key::Function),
        _ => Err(format!(
            "无法识别的按键名 '{s}'。支持 rdev Key 名（如 MetaRight、Escape、KeyA、F1…）\
             或家族名（Control/Meta/Alt/Shift，匹配左右任意变体）"
        )),
    }
}

/// 三条功能通道 + 清空键的完整快捷键绑定。
#[derive(Debug, Clone)]
pub struct Bindings {
    /// 输入/提交通道：长按录音、短按确认
    pub trigger: Vec<KeySpec>,
    /// 修正/控制通道（macOS 修正，Windows 语音控制）
    pub repair: Vec<KeySpec>,
    /// 指令/修复通道（macOS 指令，Windows 语音修复）
    pub command: Vec<KeySpec>,
    /// 清空暂存条
    pub cancel: Vec<KeySpec>,
}

impl Bindings {
    /// 当前平台的默认快捷键
    pub fn platform_default() -> Self {
        #[cfg(target_os = "macos")]
        { Self::macos_default() }
        #[cfg(target_os = "windows")]
        { Self::windows_default() }
    }

    /// macOS 默认：三个右修饰键单按
    pub fn macos_default() -> Self {
        Self {
            trigger: vec![KeySpec::Exact(Key::MetaRight)],
            repair: vec![KeySpec::Exact(Key::AltGr)],
            command: vec![KeySpec::Exact(Key::ShiftRight)],
            cancel: vec![KeySpec::Exact(Key::Escape)],
        }
    }

    /// Windows 默认：三个 Win 组合键
    pub fn windows_default() -> Self {
        Self {
            trigger: vec![
                KeySpec::Family(ModFamily::Control),
                KeySpec::Family(ModFamily::Meta),
            ],
            repair: vec![
                KeySpec::Family(ModFamily::Meta),
                KeySpec::Family(ModFamily::Alt),
            ],
            command: vec![
                KeySpec::Family(ModFamily::Meta),
                KeySpec::Family(ModFamily::Shift),
            ],
            cancel: vec![KeySpec::Exact(Key::Escape)],
        }
    }
}

// ── HotkeySource trait ───────────────────────────────────────────

/// 全局热键来源（平台抽象）
pub trait HotkeySource: Send {
    /// 启动监听（内部自行开线程），事件经 `tx` 送出。
    fn start(self: Box<Self>, tx: mpsc::Sender<HotkeyEvent>, bindings: Bindings) -> Result<()>;
    /// 所需系统权限是否已授予（macOS：辅助功能）
    fn permission_trusted(&self) -> bool;
}

/// 当前平台的默认热键实现
#[cfg(target_os = "macos")]
pub fn default_source() -> Box<dyn HotkeySource> {
    Box::new(macos::RdevHotkey)
}

#[cfg(target_os = "windows")]
pub fn default_source() -> Box<dyn HotkeySource> {
    Box::new(windows::WindowsHotkey)
}
