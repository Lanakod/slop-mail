#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod api;
mod attachments;
mod delivery;
mod desktop;
mod discovery;
mod mime;
mod network;
mod store;
fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            #[cfg(windows)]
            {
                use tauri::Manager;
                app.get_webview_window("main")
                    .expect("main window")
                    .set_decorations(false)?;
            }
            let _ = app;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            api::request,
            desktop::update,
            desktop::notify_mail
        ])
        .run(tauri::generate_context!())
        .expect("Не удалось запустить Slop Mail");
}
#[cfg(test)]
mod tests;
