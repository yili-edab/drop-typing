//! 文字注入抽象（平台相关）。
//!
//! PRD 6.3：粘贴模式（定稿）——
//!   暂存条文本 → 写入系统剪贴板 → 模拟 Cmd+V → 恢复原剪贴板
//! macOS 按键事件无法表达汉字，模拟按键模式已否决。
//!
//! Windows 移植：实现 `Injector`（Ctrl+V），在 `default_injector()` 加 cfg 分支。

#[cfg(target_os = "macos")]
pub mod macos;

use anyhow::Result;

pub trait Injector: Send + Sync {
    /// 将文本粘贴到当前聚焦 App 的光标处
    fn paste_text(&self, text: &str) -> Result<()>;
}

/// 当前平台的默认注入实现
#[cfg(target_os = "macos")]
pub fn default_injector(app: tauri::AppHandle) -> Box<dyn Injector> {
    Box::new(macos::MacosInjector::new(app))
}
