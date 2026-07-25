//! macOS 光标（文本插入点）屏幕位置查询。
//!
//! 用途：暂存条贴光标显示。查询失败（无文本焦点、目标 App 的
//! Accessibility 支持不全、权限未授予）返回 None，调用方回退底部居中。
//!
//! AX 符号沿用 hotkey/macos.rs 的手写 extern "C" 模式，不引新 crate。
//! 属性名常量（kAXFocusedUIElementAttribute 等）不链接 extern 静态符号——
//! 它们定义在 HIServices 子框架，部分链接环境下解析不到；改为用字面量
//! 构造 CFString（常量值本就是这些字符串）。
//!
//! 回退链：
//! 1. AXFocusedUIElement → AXSelectedTextRange → AXBoundsForRange（精确光标）
//! 2. 聚焦元素自身的 AXPosition + AXSize（显示在元素下方）
//! 3. 聚焦应用的 AXFocusedWindow（窗口内部底部居中）
//!
//! Electron 应用（VSCode 等）默认不暴露完整文本 AX 信息，查询前先给
//! 聚焦应用设置 AXEnhancedUserInterface（通行做法，幂等）。
//!
//! 返回坐标为屏幕 point（左上角原点），调用方按显示器 scale factor
//! 换算物理像素。

use core_foundation::base::{CFRelease, TCFType};
use core_foundation::base::CFTypeRef;
use core_foundation::boolean::CFBoolean;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::geometry::{CGPoint, CGRect, CGSize};

// AXValueType（AXValue.h）
const K_AX_VALUE_CG_POINT_TYPE: u32 = 1;
const K_AX_VALUE_CG_SIZE_TYPE: u32 = 2;
const K_AX_VALUE_CG_RECT_TYPE: u32 = 3;

#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateSystemWide() -> CFTypeRef;
    fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        attribute: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: CFTypeRef,
        attribute: CFStringRef,
        value: CFTypeRef,
    ) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: CFTypeRef,
        attribute: CFStringRef,
        parameter: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXValueGetValue(value: CFTypeRef, the_type: u32, value_ptr: *mut std::ffi::c_void) -> bool;
}

/// 构造属性名 CFString（值与 kAX*Attribute 常量相同）
fn attr(name: &'static str) -> CFString {
    CFString::from_static_string(name)
}

/// 查询结果锚点
pub enum CaretAnchor {
    /// 精确位置（光标或聚焦元素 frame）：暂存条显示在其左下角
    Precise(CGRect),
    /// 只能拿到聚焦窗口 frame：暂存条在窗口内部底部居中
    Window(CGRect),
}

/// 查询当前光标锚点。取不到返回 None。
pub fn caret_anchor() -> Option<CaretAnchor> {
    unsafe {
        if !AXIsProcessTrusted() {
            return None;
        }
        let system_wide = AXUIElementCreateSystemWide();
        if system_wide.is_null() {
            return None;
        }
        enable_enhanced_ax(system_wide);
        let result = caret_anchor_inner(system_wide);
        CFRelease(system_wide);
        result
    }
}

/// 给聚焦应用打开 AXEnhancedUserInterface（Electron 应用需要；
/// 对原生应用无副作用，设置失败忽略）。
unsafe fn enable_enhanced_ax(system_wide: CFTypeRef) {
    let attr_app = attr("AXFocusedApplication");
    let mut app_el: CFTypeRef = std::ptr::null();
    if AXUIElementCopyAttributeValue(system_wide, attr_app.as_concrete_TypeRef(), &mut app_el) == 0
        && !app_el.is_null()
    {
        let attr_enhanced = attr("AXEnhancedUserInterface");
        let _ = AXUIElementSetAttributeValue(
            app_el,
            attr_enhanced.as_concrete_TypeRef(),
            CFBoolean::true_value().as_CFTypeRef(),
        );
        CFRelease(app_el);
    }
}

unsafe fn caret_anchor_inner(system_wide: CFTypeRef) -> Option<CaretAnchor> {
    let attr_focused = attr("AXFocusedUIElement");
    let mut focused: CFTypeRef = std::ptr::null();
    if AXUIElementCopyAttributeValue(
        system_wide,
        attr_focused.as_concrete_TypeRef(),
        &mut focused,
    ) == 0
        && !focused.is_null()
    {
        // 1. 精确光标（选区矩形）；2. 聚焦元素自身 frame
        let result = caret_rect_of_focused(focused)
            .or_else(|| element_rect(focused))
            .map(CaretAnchor::Precise);
        CFRelease(focused);
        if result.is_some() {
            return result;
        }
    }
    // 3. 回退：聚焦窗口
    window_anchor(system_wide)
}

unsafe fn caret_rect_of_focused(focused: CFTypeRef) -> Option<CGRect> {
    let attr_range = attr("AXSelectedTextRange");
    let mut range: CFTypeRef = std::ptr::null();
    if AXUIElementCopyAttributeValue(focused, attr_range.as_concrete_TypeRef(), &mut range) != 0
        || range.is_null()
    {
        return None;
    }
    let attr_bounds = attr("AXBoundsForRange");
    let mut bounds: CFTypeRef = std::ptr::null();
    let ok = AXUIElementCopyParameterizedAttributeValue(
        focused,
        attr_bounds.as_concrete_TypeRef(),
        range,
        &mut bounds,
    ) == 0 && !bounds.is_null();
    CFRelease(range);
    if !ok {
        return None;
    }
    let mut rect = CGRect::new(&CGPoint::new(0.0, 0.0), &CGSize::new(0.0, 0.0));
    let got = AXValueGetValue(bounds, K_AX_VALUE_CG_RECT_TYPE, &mut rect as *mut _ as *mut _);
    CFRelease(bounds);
    // 光标是插入点时宽度可为 0，但高度（行高）必须有效——
    // 高度为 0 视为目标 App 返回的垃圾值（部分 Electron 应用如此）
    (got && rect.size.height > 0.0).then_some(rect)
}

/// 元素的 AXPosition + AXSize。尺寸无效返回 None。
unsafe fn element_rect(element: CFTypeRef) -> Option<CGRect> {
    let attr_pos = attr("AXPosition");
    let mut pos_v: CFTypeRef = std::ptr::null();
    if AXUIElementCopyAttributeValue(element, attr_pos.as_concrete_TypeRef(), &mut pos_v) != 0
        || pos_v.is_null()
    {
        return None;
    }
    let mut pos = CGPoint::new(0.0, 0.0);
    let got_pos = AXValueGetValue(pos_v, K_AX_VALUE_CG_POINT_TYPE, &mut pos as *mut _ as *mut _);
    CFRelease(pos_v);
    if !got_pos {
        return None;
    }

    let attr_size = attr("AXSize");
    let mut size = CGSize::new(0.0, 0.0);
    let mut size_v: CFTypeRef = std::ptr::null();
    if AXUIElementCopyAttributeValue(element, attr_size.as_concrete_TypeRef(), &mut size_v) == 0
        && !size_v.is_null()
    {
        let _ = AXValueGetValue(size_v, K_AX_VALUE_CG_SIZE_TYPE, &mut size as *mut _ as *mut _);
        CFRelease(size_v);
    }
    (size.width > 0.0 && size.height > 0.0).then_some(CGRect::new(&pos, &size))
}

/// 回退：聚焦应用的聚焦窗口 frame
unsafe fn window_anchor(system_wide: CFTypeRef) -> Option<CaretAnchor> {
    let attr_app = attr("AXFocusedApplication");
    let mut app_el: CFTypeRef = std::ptr::null();
    if AXUIElementCopyAttributeValue(system_wide, attr_app.as_concrete_TypeRef(), &mut app_el) != 0
        || app_el.is_null()
    {
        return None;
    }
    let attr_win = attr("AXFocusedWindow");
    let mut win_el: CFTypeRef = std::ptr::null();
    let ok = AXUIElementCopyAttributeValue(app_el, attr_win.as_concrete_TypeRef(), &mut win_el) == 0
        && !win_el.is_null();
    CFRelease(app_el);
    if !ok {
        return None;
    }
    let rect = element_rect(win_el);
    CFRelease(win_el);
    rect.map(CaretAnchor::Window)
}
