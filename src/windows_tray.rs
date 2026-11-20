use crate::desktop::show_main;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, Manager, WebviewWindow,
};

struct Quitting(AtomicBool);

pub fn is_quitting(window: &WebviewWindow) -> bool {
    window.state::<Quitting>().0.load(Ordering::SeqCst)
}

pub fn setup(app: &App) -> anyhow::Result<()> {
    app.manage(Quitting(AtomicBool::new(false)));
    let window = app.get_webview_window("main").expect("main window");
    window.set_decorations(false)?;
    let open = MenuItem::with_id(app, "show_mail", "Открыть Slop Mail", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit_mail", "Выйти", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    TrayIconBuilder::with_id("mail")
        .icon(app.default_window_icon().expect("bundled icon").clone())
        .tooltip("Slop Mail")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                show_main(tray.app_handle());
            }
        })
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show_mail" => show_main(app),
            "quit_mail" => {
                if let Some(window) = app.get_webview_window("main") {
                    let quitting = app.state::<Quitting>();
                    quitting.0.store(true, Ordering::SeqCst);
                    // Use normal close handling so the composer can flush pending drafts.
                    if window.close().is_err() {
                        quitting.0.store(false, Ordering::SeqCst);
                    }
                }
            }
            _ => {}
        })
        .build(app)?;
    Ok(())
}
