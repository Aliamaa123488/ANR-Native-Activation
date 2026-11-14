#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod monitoring;
mod native;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(monitoring::MonitorState::default())
        .invoke_handler(tauri::generate_handler![
            native::choose_game_folder,
            native::get_gpus,
            native::get_vc_runtime_status,
            native::open_vc_runtime_download,
            native::inspect_game_folder,
            native::get_latest_release,
            native::install_anr,
            native::launch_game,
            native::uninstall_anr,
            monitoring::get_monitor_snapshot,
            monitoring::set_monitor_preferences,
            monitoring::start_monitoring,
            monitoring::stop_monitoring,
            monitoring::set_overlay_visible
        ])
        .run(tauri::generate_context!())
        .expect("error while running ANR Native Activation");
}
