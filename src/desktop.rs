use tauri::Emitter;
use tauri::{AppHandle, Manager};
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
pub fn notify_update(app: AppHandle, version: String) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    app.notification()
        .builder()
        .title(format!("Доступен Slop Mail {version}"))
        .body("Откройте настройки клиента для установки обновления.")
        .show()
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub fn notify_mail(
    app: AppHandle,
    profile: String,
    account: String,
    count: usize,
) -> Result<(), String> {
    let c = crate::store::db(&profile).map_err(|e| e.to_string())?;
    let (id,thread,address,subject):(i64,String,String,String)=c.query_row("SELECT m.id,m.thread,a.address,m.subject FROM arrivals n JOIN messages m ON m.id=n.message JOIN accounts a ON a.id=n.account WHERE a.id=?",[account],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).map_err(|e|e.to_string())?;
    let mode = crate::api::run("notice_mode", &profile, serde_json::Value::Null)
        .map_err(|e| e.to_string())?;
    let title = if mode == "count" {
        format!("Slop Mail · новых писем: {count}")
    } else {
        format!("{address} · новых писем: {count}")
    };
    let subject = if mode == "full" {
        subject
    } else {
        "Открыть входящие".into()
    };
    let open = move || {
        show_main(&app);
        let _ = app.emit(
            "open-mail",
            serde_json::json!({"id":id,"thread":thread,"profile":profile}),
        );
    };
    #[cfg(windows)]
    tauri_winrt_notification::Toast::new("org.slopmail.desktop")
        .title(&title)
        .text1(&subject)
        .on_activated(move |_| {
            open();
            Ok(())
        })
        .show()
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    std::thread::spawn(move || {
        use mac_notification_sys::{send_notification, Notification, NotificationResponse};
        if matches!(
            send_notification(
                &title,
                None,
                &subject,
                Some(Notification::new().wait_for_click(true))
            ),
            Ok(NotificationResponse::Click)
        ) {
            open();
        }
    });
    Ok(())
}
#[tauri::command]
pub fn tray_counts(_app: AppHandle, summary: String) {
    #[cfg(windows)]
    if let Some(tray) = _app.tray_by_id("mail") {
        let _ = tray.set_tooltip(Some(summary.chars().take(120).collect::<String>()));
    }
    let _ = summary;
}
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[tauri::command]
pub async fn compose_window(
    app: AppHandle,
    profile: String,
    slot: String,
    to: String,
    subject: String,
    body: String,
) -> Result<(), String> {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mut hash = DefaultHasher::new();
    (&profile, &slot).hash(&mut hash);
    let label = format!("compose-{:x}", hash.finish());
    if let Some(window) = app.get_webview_window(&label) {
        window
            .show()
            .and_then(|_| window.unminimize())
            .and_then(|_| window.set_focus())
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    let context =
        serde_json::json!({ "profile":profile,"slot":slot,"to":to,"subject":subject,"body":body });
    tauri::WebviewWindowBuilder::new(&app, label, tauri::WebviewUrl::App("index.html".into()))
        .initialization_script(format!("window.__composeContext = {context};"))
        .title("Новое письмо · Slop Mail")
        .inner_size(760., 670.)
        .min_inner_size(440., 380.)
        .build()
        .map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
pub fn finish_close(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.label().starts_with("compose-") {
        return window.destroy().map_err(|e| e.to_string());
    }
    #[cfg(windows)]
    if crate::windows_tray::is_quitting(&window) {
        return window.destroy().map_err(|e| e.to_string());
    }
    window.hide().map_err(|e| e.to_string())
}
