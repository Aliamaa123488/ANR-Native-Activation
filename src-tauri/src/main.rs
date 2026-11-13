#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod native;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            native::choose_game_folder,
            native::get_gpus,
            native::get_vc_runtime_status,
            native::open_vc_runtime_download,
            native::inspect_game_folder,
            native::get_latest_release,
            native::install_anr,
            native::launch_game,
            native::uninstall_anr
        ])
        .run(tauri::generate_context!())
        .expect("error while running ANR Native Activation");
}
