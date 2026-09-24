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

// ---------------------------------------------------------------------------
// Связь между экземплярами: второй запуск (PrintScreen через Windows, двойной
// клик по .frost) передаёт аргументы уже работающему Frostshot и выходит.
// Windows: файл команды + именованное событие. Другие ОС пока без этого.
// ---------------------------------------------------------------------------

fn ipc_dir() -> Option<std::path::PathBuf> {
    let d = directories::ProjectDirs::from("", "", "Frostshot")?.data_local_dir().join("ipc");
    Some(if cfg!(debug_assertions) { d.join("dev") } else { d })
}

#[cfg(windows)]
fn ipc_event_name() -> Vec<u16> {
    let n = if cfg!(debug_assertions) { "Local\\Frostshot-dev-ipc-7c1e" } else { "Local\\Frostshot-ipc-7c1e" };
    n.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Слушать команды второго экземпляра. on_cmd вызывается из фонового потока.
pub fn ipc_listen(on_cmd: impl Fn(Vec<String>) + Send + 'static) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{CreateEventW, INFINITE, WaitForSingleObject};
        let Some(dir) = ipc_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        // Команды, оставшиеся от прошлого запуска, не выполняем.
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                let _ = std::fs::remove_file(e.path());
            }
        }
        let name = ipc_event_name();
        // SAFETY: имя завершено нулём; событие с автосбросом живёт до конца процесса.
        let ev = unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) };
        if ev.is_null() {
            log::warn!("ipc event not created");
            return;
        }
        let ev = ev as usize;
        std::thread::spawn(move || {
            loop {
                // SAFETY: хэндл события валиден до конца процесса.
                unsafe { WaitForSingleObject(ev as windows_sys::Win32::Foundation::HANDLE, INFINITE) };
                let Ok(rd) = std::fs::read_dir(&dir) else { continue };
                let mut files: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "cmd")).collect();
                files.sort();
                for f in files {
                    if let Ok(text) = std::fs::read_to_string(&f) {
                        let _ = std::fs::remove_file(&f);
                        on_cmd(text.lines().map(str::to_string).collect());
                    }
                }
            }
        });
    }
    #[cfg(not(windows))]
    let _ = on_cmd;
}

/// Передать аргументы работающему экземпляру. false: передать не удалось.
pub fn ipc_send(args: &[String]) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{EVENT_MODIFY_STATE, OpenEventW, SetEvent};
        let Some(dir) = ipc_dir() else { return false };
        if std::fs::create_dir_all(&dir).is_err() {
            return false;
        }
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let file = dir.join(format!("{stamp}-{}.cmd", std::process::id()));
        if std::fs::write(&file, args.join("\n")).is_err() {
            return false;
        }
        let name = ipc_event_name();
        // SAFETY: имя завершено нулём, хэндл закрываем.
        unsafe {
            let ev = OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr());
            if ev.is_null() {
                let _ = std::fs::remove_file(&file);
                return false;
            }
            SetEvent(ev);
            CloseHandle(ev);
        }
        true
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        false
    }
}

// ---------------------------------------------------------------------------
// Интеграция с Windows: Frostshot как обработчик ms-screenclip (PrintScreen и
// Win+Shift+S при включённом параметре Windows) и открытие .frost двойным кликом.
// Только HKCU, без прав администратора. Выбор программы по умолчанию делает
// пользователь в «Приложениях по умолчанию»: Windows не даёт менять его из кода.
// ---------------------------------------------------------------------------

pub const SCREENCLIP_PROGID: &str = "Frostshot.ScreenClip";

#[derive(Clone, Debug, Default)]
pub struct ShellStatus {
    /// Поддерживается ли интеграция на этой ОС.
    pub supported: bool,
    /// Frostshot записан в список обработчиков.
    pub registered: bool,
    /// ProgId, который Windows открывает по PrintScreen (None: Ножницы Windows).
    pub handler: Option<String>,
    /// Включён ли параметр Windows «PrintScreen открывает захват экрана».
    pub key_enabled: bool,
}

impl ShellStatus {
    pub fn frostshot_is_handler(&self) -> bool {
        self.handler.as_deref() == Some(SCREENCLIP_PROGID)
    }
}

#[cfg(windows)]
mod winreg {
    use windows_sys::Win32::System::Registry::*;

    pub fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn set_str(path: &str, name: Option<&str>, value: &str) -> Result<(), String> {
        let mut key: HKEY = std::ptr::null_mut();
        let p = w(path);
        // SAFETY: строки завершены нулём, ключ закрывается.
        unsafe {
            let rc = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                p.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_ALL_ACCESS,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            );
            if rc != 0 {
                return Err(format!("реестр {path}: код {rc}"));
            }
            let n = name.map(w);
            let data = w(value);
            let rc = RegSetValueExW(
                key,
                n.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            );
            RegCloseKey(key);
            if rc != 0 {
                return Err(format!("реестр {path}: код {rc}"));
            }
        }
        Ok(())
    }

    pub fn get_str(root: HKEY, path: &str, name: Option<&str>) -> Option<String> {
        let p = w(path);
        let n = name.map(w);
        let mut buf = vec![0u16; 1024];
        let mut size = (buf.len() * 2) as u32;
        // SAFETY: буфер size байт, строки завершены нулём.
        let rc = unsafe {
            RegGetValueW(
                root,
                p.as_ptr(),
                n.as_ref().map_or(std::ptr::null(), |v| v.as_ptr()),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if rc != 0 {
            return None;
        }
        buf.truncate((size as usize / 2).saturating_sub(1));
        Some(String::from_utf16_lossy(&buf))
    }

    pub fn delete_tree(path: &str) {
        let p = w(path);
        // SAFETY: строка завершена нулём. RegDeleteTreeW удаляет содержимое, затем сам ключ.
        unsafe {
            RegDeleteTreeW(HKEY_CURRENT_USER, p.as_ptr());
            RegDeleteKeyW(HKEY_CURRENT_USER, p.as_ptr());
        }
    }

    pub fn delete_value(path: &str, name: &str) {
        let (p, n) = (w(path), w(name));
        // SAFETY: строки завершены нулём.
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, p.as_ptr(), n.as_ptr()) };
    }
}

pub fn shell_status() -> ShellStatus {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Registry::HKEY_CURRENT_USER;
        let registered = winreg::get_str(HKEY_CURRENT_USER, "Software\\RegisteredApplications", Some("Frostshot")).is_some();
        let handler = winreg::get_str(
            HKEY_CURRENT_USER,
            "Software\\Microsoft\\Windows\\Shell\\Associations\\UrlAssociations\\ms-screenclip\\UserChoice",
            Some("ProgId"),
        );
        ShellStatus { supported: true, registered, handler, key_enabled: printscreen_taken_by_system() }
    }
    #[cfg(not(windows))]
    {
        ShellStatus::default()
    }
}

/// Записать или убрать Frostshot из обработчиков ms-screenclip и .frost.
pub fn shell_register(on: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let exe = exe.to_string_lossy().to_string();
        let icon = format!("\"{exe}\",0");
        if on {
            let c = "Software\\Classes";
            let clip = format!("{c}\\{SCREENCLIP_PROGID}");
            winreg::set_str(&clip, None, "Frostshot: снимок экрана")?;
            winreg::set_str(&format!("{clip}\\Application"), Some("ApplicationName"), "Frostshot")?;
            winreg::set_str(&format!("{clip}\\Application"), Some("ApplicationIcon"), &icon)?;
            winreg::set_str(&format!("{clip}\\DefaultIcon"), None, &icon)?;
            winreg::set_str(&format!("{clip}\\shell\\open\\command"), None, &format!("\"{exe}\" --capture \"%1\""))?;
            let proj = format!("{c}\\Frostshot.Project");
            winreg::set_str(&proj, None, "Проект Frostshot")?;
            winreg::set_str(&format!("{proj}\\DefaultIcon"), None, &icon)?;
            winreg::set_str(&format!("{proj}\\shell\\open\\command"), None, &format!("\"{exe}\" \"%1\""))?;
            winreg::set_str(&format!("{c}\\.frost"), None, "Frostshot.Project")?;
            let cap = "Software\\Frostshot\\Capabilities";
            winreg::set_str(cap, Some("ApplicationName"), "Frostshot")?;
            winreg::set_str(cap, Some("ApplicationDescription"), "Скриншоты с разметкой")?;
            winreg::set_str(&format!("{cap}\\URLAssociations"), Some("ms-screenclip"), SCREENCLIP_PROGID)?;
            winreg::set_str(&format!("{cap}\\FileAssociations"), Some(".frost"), "Frostshot.Project")?;
            winreg::set_str("Software\\RegisteredApplications", Some("Frostshot"), cap)?;
        } else {
            winreg::delete_value("Software\\RegisteredApplications", "Frostshot");
            winreg::delete_tree("Software\\Frostshot");
            winreg::delete_tree(&format!("Software\\Classes\\{SCREENCLIP_PROGID}"));
            winreg::delete_tree("Software\\Classes\\Frostshot.Project");
            winreg::delete_tree("Software\\Classes\\.frost");
        }
        use windows_sys::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
        // SAFETY: уведомление оболочки без параметров.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED as i32, SHCNF_IDLIST, std::ptr::null(), std::ptr::null()) };
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = on;
        Err("только Windows".into())
    }
}

/// Открыть в Windows страницу выбора программ по умолчанию для Frostshot.
pub fn open_default_apps() {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd").args(["/C", "start", "", "ms-settings:defaultapps?registeredAppUser=Frostshot"]).spawn();
    }
}

/// Открыть в Windows параметры клавиатуры (переключатель PrintScreen).
pub fn open_keyboard_settings() {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd").args(["/C", "start", "", "ms-settings:easeofaccess-keyboard"]).spawn();
    }
}

#[cfg(windows)]
fn foreground_class() -> (windows_sys::Win32::Foundation::HWND, String) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow};
    // SAFETY: чтение класса окна в локальный буфер.
    unsafe {
        let h = GetForegroundWindow();
        if h.is_null() {
            return (h, String::new());
        }
        let mut buf = [0u16; 128];
        let n = GetClassNameW(h, buf.as_mut_ptr(), buf.len() as i32);
        (h, String::from_utf16_lossy(&buf[..n.max(0) as usize]))
    }
}

/// Меню «Пуск», поиск и центр уведомлений живут в слое выше «поверх всех» окон,
/// и активировать своё окно, пока они открыты, Windows не даёт. Вызывать после
/// снимка экрана: если активна такая панель оболочки, закрыть её Esc и дождаться,
/// пока фокус вернётся к обычному окну. Обычным программам Esc не отправляется.
pub fn dismiss_shell_flyout() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{KEYEVENTF_KEYUP, VK_ESCAPE, keybd_event};
        let (_, class) = foreground_class();
        if class != "Windows.UI.Core.CoreWindow" {
            return;
        }
        // SAFETY: парное нажатие и отпускание клавиши.
        unsafe {
            keybd_event(VK_ESCAPE as u8, 0, 0, 0);
            keybd_event(VK_ESCAPE as u8, 0, KEYEVENTF_KEYUP, 0);
        }
        let t0 = std::time::Instant::now();
        while t0.elapsed() < std::time::Duration::from_millis(400) {
            std::thread::sleep(std::time::Duration::from_millis(15));
            if foreground_class().1 != class {
                break;
            }
        }
        log::info!("shell flyout closed in {:?}, foreground now {}", t0.elapsed(), foreground_class().1);
    }
}

/// Сделать окно активным. Windows запрещает фоновой программе забирать фокус,
/// поэтому на время подключаемся к очереди ввода активной программы.
pub fn force_foreground(window: &winit::window::Window) {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{SetActiveWindow, SetFocus};
        use windows_sys::Win32::UI::WindowsAndMessaging::{BringWindowToTop, GetForegroundWindow, GetWindowThreadProcessId, SetForegroundWindow};
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let Ok(h) = window.window_handle() else { return window.focus_window() };
        let RawWindowHandle::Win32(w) = h.as_raw() else { return window.focus_window() };
        let hwnd = w.hwnd.get() as windows_sys::Win32::Foundation::HWND;
        // SAFETY: hwnd живого окна; присоединение ввода снимаем в этой же функции.
        unsafe {
            let (fg, fg_class) = foreground_class();
            let fg_tid = if fg.is_null() { 0 } else { GetWindowThreadProcessId(fg, std::ptr::null_mut()) };
            let me = GetCurrentThreadId();
            let attached = fg_tid != 0 && fg_tid != me && AttachThreadInput(me, fg_tid, 1) != 0;
            BringWindowToTop(hwnd);
            SetForegroundWindow(hwnd);
            SetActiveWindow(hwnd);
            SetFocus(hwnd);
            if attached {
                AttachThreadInput(me, fg_tid, 0);
            }
            let ok = GetForegroundWindow() == hwnd;
            log::info!("overlay foreground: {} (was {fg_class}, attached {attached})", if ok { "ok" } else { "FAILED" });
        }
        return;
    }
    #[cfg(not(windows))]
    window.focus_window();
}

/// Распознать текст на картинке (RGBA premultiplied, непрозрачная). Windows: Windows.Media.Ocr,
/// локально, языки из профиля пользователя. Блокирующий вызов: только из фонового потока.
pub fn ocr_recognize(img: &tiny_skia::Pixmap) -> Result<Vec<crate::ocr::Line>, String> {
    #[cfg(windows)]
    {
        use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
        use windows::Media::Ocr::OcrEngine;
        use windows::Storage::Streams::DataWriter;
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        let e = |e: windows::core::Error| format!("OCR: {}", e.message());
        // SAFETY: инициализация WinRT для текущего (фонового) потока.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| "Нет языка распознавания: добавьте пакет OCR в «Язык и регион» Windows".to_string())?;
        let max = OcrEngine::MaxImageDimension().unwrap_or(4096) as f32;
        // Мелкий текст распознаётся лучше, если картинку увеличить; большие уменьшаем до предела.
        let (w, h) = (img.width() as f32, img.height() as f32);
        let mut k = if w.max(h) * 2.0 <= max { 2.0 } else { 1.0 };
        if w.max(h) * k > max {
            k = max / w.max(h);
        }
        let (sw, sh) = (((w * k).round() as u32).max(1), ((h * k).round() as u32).max(1));
        let mut scaled = tiny_skia::Pixmap::new(sw, sh).ok_or("OCR: пустая картинка")?;
        let paint = tiny_skia::PixmapPaint { quality: tiny_skia::FilterQuality::Bicubic, ..Default::default() };
        scaled.draw_pixmap(0, 0, img.as_ref(), &paint, tiny_skia::Transform::from_scale(k, k), None);
        let mut bgra = scaled.data().to_vec();
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let writer = DataWriter::new().map_err(e)?;
        writer.WriteBytes(&bgra).map_err(e)?;
        let buffer = writer.DetachBuffer().map_err(e)?;
        let bmp = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(&buffer, BitmapPixelFormat::Bgra8, sw as i32, sh as i32, BitmapAlphaMode::Premultiplied)
            .map_err(e)?;
        let result = engine.RecognizeAsync(&bmp).map_err(e)?.join().map_err(e)?;
        let mut lines = Vec::new();
        for line in result.Lines().map_err(e)? {
            let mut words = Vec::new();
            for word in line.Words().map_err(e)? {
                let r = word.BoundingRect().map_err(e)?;
                words.push(crate::ocr::Word {
                    text: word.Text().map_err(e)?.to_string(),
                    rect: (r.X / k, r.Y / k, r.Width / k, r.Height / k),
                });
            }
            lines.push(crate::ocr::Line { words });
        }
        Ok(lines)
    }
    #[cfg(not(windows))]
    {
        let _ = img;
        Err("Распознавание текста пока есть только в Windows".into())
    }
}
