mod commands;
pub mod domain;
pub mod error;
pub mod protocol;
pub mod resources;
pub mod sidecar;

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
            commands::save_processing_config,
            commands::refresh_processing_config,
            commands::preview_rules,
            commands::list_models,
            commands::start_sync,
            commands::cancel_sync,
            commands::get_run_summary,
            commands::audit_location,
            commands::open_audit_folder,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                tauri::async_runtime::block_on(
                    app.state::<commands::AppState>().shutdown_sidecar(),
                );
            }
        });
}
