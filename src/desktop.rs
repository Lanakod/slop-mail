use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;
#[tauri::command]
pub async fn update(
    app: AppHandle,
    install_version: Option<String>,
) -> Result<Option<String>, String> {
    let updater = app
        .updater_builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let Some(release) = updater
        .check()
        .await
        .map_err(|e| format!("Не удалось получить манифест GitHub Release: {e}"))?
    else {
        return Ok(None);
    };
    if let Some(expected) = install_version {
        if expected != release.version {
            return Err("Доступная версия изменилась. Повторите проверку.".into());
        }
        release
            .download_and_install(|_, _| {}, || {})
            .await
            .map_err(|e| e.to_string())?;
        app.restart();
    }
    Ok(Some(release.version))
}
#[tauri::command]
pub fn notify_mail(app: AppHandle, account: String, count: usize) -> Result<(), String> {
    app.notification()
        .builder()
        .title("Новая почта")
        .body(format!("{account} · Новых писем: {count}"))
        .show()
        .map_err(|e| e.to_string())
}
