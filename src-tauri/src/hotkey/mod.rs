//! 全局热键抽象（平台相关）。
//!
//! M1 选型说明：
//! - tauri-plugin-global-shortcut 面向"组合键按下即触发"，
//!   对"裸右 ⌘ 单独按下 + press/release 事件 + 时长判定"支持不足，故未采用。
//! - macOS 实现使用 rdev 的全局事件监听（CGEventTap），
//!   可精确拿到 RightMeta 的 press / release 事件。需要辅助功能权限。
//!
//! Windows 移植：实现 `HotkeySource`（如映射 Right Win / Right Alt / Right Shift，PRD 第 5 章），
//! 在 `default_source()` 中加 cfg 分支即可。

#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

use std::sync::mpsc;

use anyhow::Result;

/// 热键事件。时长判定（短按/长按）放在 pipeline 做，本层只报原始按下/松开。
#[derive(Debug, Clone)]
pub enum HotkeyEvent {
    /// 右 ⌘ 按下（输入/提交通道）
    TriggerDown,
    /// 右 ⌘ 松开
    TriggerUp,
    /// 右 ⌥ 按下（修正通道）
    RepairDown,
    /// 右 ⌥ 松开
    RepairUp,
    /// 右 ⇧ 按下（指令通道，M4）
    CommandDown,
    /// 右 ⇧ 松开
    CommandUp,
    /// 录音期间有其它键按下（说明右修饰键被当作组合键修饰键使用，应当作废本次录音）
    OtherKeyDown,
    /// 监听器运行时错误（如权限被收回）
    Error(String),
}

/// 全局热键来源（平台抽象）
pub trait HotkeySource: Send {
    /// 启动监听（内部自行开线程），事件经 `tx` 送出。
    fn start(self: Box<Self>, tx: mpsc::Sender<HotkeyEvent>) -> Result<()>;
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
