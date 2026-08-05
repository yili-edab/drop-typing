use crate::rdev::{Event, EventType, ListenError};
use crate::windows::common::{convert, get_code, set_key_hook, set_mouse_hook, HookError, HOOK, KEYBOARD};
use std::os::raw::c_int;
use std::ptr::null_mut;
use std::time::SystemTime;
use winapi::shared::minwindef::{LPARAM, LRESULT, WPARAM};
use winapi::um::winuser::{CallNextHookEx, GetMessageA, HC_ACTION, WM_KEYDOWN, WM_KEYUP, WM_SYSKEYDOWN, WM_SYSKEYUP};

static mut GLOBAL_CALLBACK: Option<Box<dyn FnMut(Event)>> = None;

use std::sync::atomic::{AtomicBool, Ordering};

/// drop-typing：标记下一次 Win 键「按下」需要被吞掉（属于 drop-typing 组合）。
static SWALLOW_WIN_DOWN: AtomicBool = AtomicBool::new(false);
/// drop-typing：标记下一次 Win 键「松开」需要被吞掉（属于 drop-typing 组合）。
static SWALLOW_WIN_UP: AtomicBool = AtomicBool::new(false);

pub fn set_swallow_win_down(swallow: bool) {
    SWALLOW_WIN_DOWN.store(swallow, Ordering::SeqCst);
}

pub fn set_swallow_win_up(swallow: bool) {
    SWALLOW_WIN_UP.store(swallow, Ordering::SeqCst);
}

impl From<HookError> for ListenError {
    fn from(error: HookError) -> Self {
        match error {
            HookError::Mouse(code) => ListenError::MouseHookError(code),
            HookError::Key(code) => ListenError::KeyHookError(code),
        }
    }
}

unsafe extern "system" fn raw_callback(code: c_int, param: WPARAM, lpdata: LPARAM) -> LRESULT {
    if code == HC_ACTION {
        // drop-typing: 用 param（Windows 消息 ID）直接判断键盘事件，不依赖
        // convert() 返回的 EventType 模式匹配。param 是 WM_KEYDOWN/WM_KEYUP/
        // WM_SYSKEYDOWN/WM_SYSKEYUP 之一时才读 KBDLLHOOKSTRUCT 的 vkCode。
        let msg: u32 = param as u32;
        let is_win = matches!(msg, WM_KEYDOWN | WM_KEYUP | WM_SYSKEYDOWN | WM_SYSKEYUP)
            && {
                let vk = get_code(lpdata);
                vk == 0x5B || vk == 0x5C // VK_LWIN or VK_RWIN
            };

        let opt = convert(param, lpdata);
        if let Some(event_type) = opt {
            let name = match &event_type {
                EventType::KeyPress(_key) => match (*KEYBOARD).lock() {
                    Ok(mut keyboard) => keyboard.get_name(lpdata),
                    Err(_) => None,
                },
                _ => None,
            };
            let event = Event {
                event_type,
                time: SystemTime::now(),
                name,
            };
            if let Some(callback) = &mut GLOBAL_CALLBACK {
                callback(event);
            }
        }

        // drop-typing：只有 App 标记为「属于 drop-typing 组合」的 Win 键事件才拦截；
        // 其它 Win 键事件（开始菜单、Win+E、Win+R 等）放行给系统。
        if is_win {
            let down = matches!(msg, WM_KEYDOWN | WM_SYSKEYDOWN);
            let swallow = if down {
                SWALLOW_WIN_DOWN.swap(false, Ordering::SeqCst)
            } else {
                SWALLOW_WIN_UP.swap(false, Ordering::SeqCst)
            };
            if swallow {
                return 1;
            }
        }
    }
    CallNextHookEx(HOOK, code, param, lpdata)
}

pub fn listen<T>(callback: T) -> Result<(), ListenError>
where
    T: FnMut(Event) + 'static,
{
    unsafe {
        GLOBAL_CALLBACK = Some(Box::new(callback));
        set_key_hook(raw_callback)?;
        set_mouse_hook(raw_callback)?;

        GetMessageA(null_mut(), null_mut(), 0, 0);
    }
    Ok(())
}
