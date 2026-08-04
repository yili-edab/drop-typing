//! 输入设备枚举与解析（定制硬件：USB 麦克风选择）。
//!
//! 匹配策略：用户配置了设备名（如 "Drop Mic"）且当前存在同名设备时用之；
//! 找不到时静默回退系统默认输入设备（热插拔场景下自动恢复）。

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait};

/// 枚举所有输入设备，返回 `(设备名, 是否为系统默认)` 列表。
pub fn list_input_devices() -> Result<Vec<(String, bool)>> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|d| d.name().ok());
    let mut out = Vec::new();
    for dev in host.input_devices()? {
        if let Ok(name) = dev.name() {
            let name = name.trim().to_string();
            if name.is_empty() {
                continue;
            }
            let is_default = default_name.as_deref() == Some(name.as_str());
            out.push((name, is_default));
        }
    }
    Ok(out)
}

/// 解析实际使用的输入设备：
/// - `configured` 有值且设备存在 → 使用该设备；
/// - 否则 → 系统默认输入设备。
pub fn resolve_input_device(configured: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    let configured = configured
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(name) = configured {
        for dev in host.input_devices()? {
            if let Ok(n) = dev.name() {
                if n.trim() == name {
                    eprintln!("[drop-typing] 使用配置的输入设备：{name}");
                    return Ok(dev);
                }
            }
        }
        eprintln!("[drop-typing] 未找到配置的输入设备「{name}」，回退系统默认");
    }
    host.default_input_device()
        .ok_or_else(|| anyhow!("未找到可用的麦克风设备"))
}
