//! Весь платформенно-зависимый код. Остальные модули без #[cfg(target_os)].

use std::path::Path;
use winit::window::WindowAttributes;

/// Платформенные атрибуты окна-оверлея.
pub fn overlay_attributes(attrs: WindowAttributes) -> WindowAttributes {
    #[cfg(windows)]
    {
        use winit::platform::windows::WindowAttributesExtWindows;
        attrs.with_skip_taskbar(true)
    }
    #[cfg(not(windows))]
    {
        attrs
    }
}

/// Глобальная позиция курсора в физических пикселях.
pub fn cursor_pos() -> Option<(i32, i32)> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut p = POINT { x: 0, y: 0 };
        // SAFETY: p валидный указатель на POINT.
        if unsafe { GetCursorPos(&mut p) } != 0 {
            return Some((p.x, p.y));
        }
        None
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Открыть папку в файловом менеджере.
pub fn open_folder(path: &Path) {
    let _ = std::fs::create_dir_all(path);
    #[cfg(windows)]
    let cmd = "explorer";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let cmd = "xdg-open";
    if let Err(e) = std::process::Command::new(cmd).arg(path).spawn() {
        log::warn!("open folder failed: {e}");
    }
}

/// Windows 11: "Использовать PrtScn для открытия Snipping Tool" перехватывает клавишу.
pub fn printscreen_taken_by_system() -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
        let key: Vec<u16> = "Control Panel\\Keyboard\0".encode_utf16().collect();
        let val: Vec<u16> = "PrintScreenKeyForSnippingEnabled\0".encode_utf16().collect();
        let mut data: u32 = 0;
        let mut size: u32 = 4;
        // SAFETY: строки завершены нулём, буфер на 4 байта под DWORD.
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                val.as_ptr(),
                RRF_RT_REG_DWORD,
                std::ptr::null_mut(),
                (&mut data as *mut u32).cast(),
                &mut size,
            )
        };
        // Значение отсутствует: на Windows 11 по умолчанию включено.
        if rc != 0 {
            return is_windows_11();
        }
        data == 1
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(windows)]
fn is_windows_11() -> bool {
    use windows_sys::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
    let key: Vec<u16> = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\0".encode_utf16().collect();
    let val: Vec<u16> = "CurrentBuildNumber\0".encode_utf16().collect();
    let mut buf = [0u16; 32];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: буфер размера size байт.
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            val.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if rc != 0 {
        return false;
    }
    let s: String = String::from_utf16_lossy(&buf).trim_end_matches('\0').to_string();
    s.parse::<u32>().map(|b| b >= 22000).unwrap_or(false)
}

/// Пути к системным шрифтам с кириллицей (шрифт не встраиваем).
pub fn font_candidates() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &["C:\\Windows\\Fonts\\segoeui.ttf", "C:\\Windows\\Fonts\\arial.ttf"]
    }
    #[cfg(target_os = "macos")]
    {
        &[
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/Library/Fonts/Arial.ttf",
            "/System/Library/Fonts/Helvetica.ttc",
        ]
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        &[
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/TTF/DejaVuSans.ttf",
            "/usr/share/fonts/dejavu/DejaVuSans.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
            "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
        ]
    }
}

fn auto_launch() -> Result<auto_launch::AutoLaunch, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut b = auto_launch::AutoLaunchBuilder::new();
    b.set_app_name("Frostshot").set_app_path(&exe.to_string_lossy());
    #[cfg(windows)]
    b.set_windows_enable_mode(auto_launch::WindowsEnableMode::CurrentUser);
    #[cfg(target_os = "macos")]
    b.set_macos_launch_mode(auto_launch::MacOSLaunchMode::LaunchAgent);
    b.build().map_err(|e| e.to_string())
}

/// Включить или выключить запуск при входе в систему (без прав администратора).
/// Включение всегда перезаписывает путь на текущий exe.
pub fn set_autostart(on: bool) -> Result<(), String> {
    let al = auto_launch()?;
    if on {
        al.enable().map_err(|e| e.to_string())
    } else if al.is_enabled().unwrap_or(false) {
        al.disable().map_err(|e| e.to_string())
    } else {
        Ok(())
    }
}

pub fn autostart_enabled() -> bool {
    auto_launch().and_then(|a| a.is_enabled().map_err(|e| e.to_string())).unwrap_or(false)
}

/// Открыть файл в приложении по умолчанию.
pub fn open_file(path: &Path) {
    #[cfg(windows)]
    let r = std::process::Command::new("cmd").args(["/C", "start", ""]).arg(path).spawn();
    #[cfg(target_os = "macos")]
    let r = std::process::Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let r = std::process::Command::new("xdg-open").arg(path).spawn();
    if let Err(e) = r {
        log::warn!("open file failed: {e}");
    }
}

/// Файл лога рядом с данными приложения.
pub fn log_path() -> Option<std::path::PathBuf> {
    directories::ProjectDirs::from("", "", "Frostshot").map(|d| d.data_local_dir().join("frostshot.log"))
}

/// Рабочая область (без панели задач) монитора, содержащего точку: l, t, r, b.
pub fn work_area(x: i32, y: i32) -> Option<(i32, i32, i32, i32)> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint};
        // SAFETY: MONITORINFO с корректным cbSize, hmonitor от MonitorFromPoint.
        unsafe {
            let hm = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(hm, &mut mi) != 0 {
                let r = mi.rcWork;
                return Some((r.left, r.top, r.right, r.bottom));
            }
        }
        None
    }
    #[cfg(not(windows))]
    {
        let _ = (x, y);
        None
    }
}

/// Окно не активируется по клику (уведомление не отбирает фокус у текущей программы).
pub fn make_no_activate(window: &winit::window::Window) {
    #[cfg(windows)]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_NOACTIVATE};
        if let Ok(h) = window.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                let hwnd = w.hwnd.get() as windows_sys::Win32::Foundation::HWND;
                // SAFETY: hwnd живого окна winit.
                unsafe {
                    let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
                    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE as isize);
                }
            }
        }
    }
    #[cfg(not(windows))]
    let _ = window;
}

/// Показать окно без активации.
pub fn show_no_activate(window: &winit::window::Window) {
    #[cfg(windows)]
    {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        use windows_sys::Win32::UI::WindowsAndMessaging::{SW_SHOWNOACTIVATE, ShowWindow};
        if let Ok(h) = window.window_handle() {
            if let RawWindowHandle::Win32(w) = h.as_raw() {
                // SAFETY: hwnd живого окна winit.
                unsafe { ShowWindow(w.hwnd.get() as windows_sys::Win32::Foundation::HWND, SW_SHOWNOACTIVATE) };
                return;
            }
        }
    }
    window.set_visible(true);
}
