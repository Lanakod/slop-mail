#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod api;
mod delivery;
mod mime;
mod network;
mod store;
fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![api::request])
        .run(tauri::generate_context!())
        .expect("Не удалось запустить Slop Mail");
}
#[cfg(test)]
mod tests;
