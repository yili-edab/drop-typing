use crate::rdev::{Event, EventType, ListenError};
use crate::windows::common::{convert, get_code, set_key_hook, set_mouse_hook, HookError, HOOK, KEYBOARD};
use std::os::raw::c_int;
use std::ptr::null_mut;
use std::time::SystemTime;
use winapi::shared::minwindef::{LPARAM, LRESULT, WPARAM};
use winapi::um::winuser::{CallNextHookEx, GetMessageA, HC_ACTION};

static mut GLOBAL_CALLBACK: Option<Box<dyn FnMut(Event)>> = None;

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
        let opt = convert(param, lpdata);

        // drop-typing: 在事件处理前判定右 Win 键（VK_RWIN = 0x5C）。
        // 键盘事件的 lpdata 是 KBDLLHOOKSTRUCT，vkCode 在首字段；
        // 鼠标事件的 lpdata 是 MSLLHOOKSTRUCT，结构不同——故先通过 opt
        // 确认是键盘事件后才读 vkCode，避免未定义行为。
        let is_rwin_key = matches!(
            &opt,
            Some(EventType::KeyPress(_) | EventType::KeyRelease(_))
        ) && get_code(lpdata) == 0x5C;

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

        // 拦截右 Win 键：返回非零阻止消息传递到系统，避免弹出开始菜单。
        // 用户回调已在上面调用，App 仍能感知按键事件。
        if is_rwin_key {
            return 1;
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
