#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod api;
mod attachments;
mod book;
mod delivery;
mod desktop;
mod discovery;
mod mime;
mod network;
mod organize;
mod scheduler;
mod store;
#[cfg(windows)]
mod windows_tray;
fn main() {
    let builder = tauri::Builder::default();
    #[cfg(windows)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _, _| {
        desktop::show_main(app);
    }));
    builder
        .manage(scheduler::Scheduler::default())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            #[cfg(windows)]
            {
                windows_tray::setup(app)?;
            }
            let _ = app;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scheduler::set_schedule,
            api::request,
            desktop::finish_close,
            desktop::compose_window,
            desktop::update,
            desktop::tray_counts,
            desktop::notify_update,
            desktop::notify_mail
        ])
        .build(tauri::generate_context!())
        .expect("Не удалось запустить Slop Mail")
        .run(|_app, _event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = _event {
                desktop::show_main(_app);
            }
        });
}
#[cfg(test)]
mod tests;
