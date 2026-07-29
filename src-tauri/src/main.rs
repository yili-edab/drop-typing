// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// Windows 单实例检测：通过全局命名互斥体判断是否已有实例在运行。
/// 若已存在则弹出提示框并退出，避免重复启动造成热键冲突等问题。
#[cfg(target_os = "windows")]
mod win_util {
    extern "system" {
        fn CreateMutexW(
            lpMutexAttributes: *mut std::ffi::c_void,
            bInitialOwner: i32,
            lpName: *const u16,
        ) -> isize;
        fn GetLastError() -> u32;
        fn MessageBoxW(
            hWnd: isize,
            lpText: *const u16,
            lpCaption: *const u16,
            uType: u32,
        ) -> i32;
    }

    const ERROR_ALREADY_EXISTS: u32 = 183;

    pub fn ensure_single_instance() {
        let name: Vec<u16> = "Global\\drop-typing-single-instance"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let _handle = CreateMutexW(std::ptr::null_mut(), 0, name.as_ptr());
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let text: Vec<u16> = "drop-typing 已在运行中，请查看系统托盘。"
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                let caption: Vec<u16> = "drop-typing"
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();
                // MB_ICONINFORMATION | MB_OK
                MessageBoxW(0, text.as_ptr(), caption.as_ptr(), 0x40);
                std::process::exit(0);
            }
        }
    }
}

fn main() {
    #[cfg(target_os = "windows")]
    win_util::ensure_single_instance();

    drop_typing_lib::run()
}
