// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(all(windows, not(debug_assertions)))]
    if prepare_runtime().is_err() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let title: Vec<u16> = "Redactio\0".encode_utf16().collect();
        let message: Vec<u16> = "Die Installation ist unvollständig oder beschädigt. Bitte das vollständige Redactio-ZIP erneut in einen lokalen Ordner entpacken. Die Ordner sidecar, models und webview2 müssen neben redactio.exe liegen.\0".encode_utf16().collect();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                message.as_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR,
            );
        }
        return;
    }
    redactio_lib::run();
}

#[cfg(all(windows, not(debug_assertions)))]
fn prepare_runtime() -> Result<(), redactio_lib::error::AppError> {
    redactio_lib::resources::resolve()?;
    let executable = std::env::current_exe()?;
    let runtime = redactio_lib::resources::webview_directory(&executable)?;
    // Single-threaded process startup, before Tauri creates any window.
    std::env::set_var("WEBVIEW2_BROWSER_EXECUTABLE_FOLDER", runtime);
    Ok(())
}
