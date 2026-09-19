mod commands;
pub mod domain;
pub mod error;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(commands::AppState::initialize(app.handle())?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_pairs,
            commands::list_recovery_pairs,
            commands::fresh_start_pair,
            commands::add_pair,
            commands::rename_pair,
            commands::select_pair,
            commands::remove_pair,
            commands::scan_pair,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
