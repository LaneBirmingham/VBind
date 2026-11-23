//! VBind desktop app: runs the engine and local API in-process and shows the
//! UI it serves in a window. Closing the window keeps mappings running from
//! the tray; Quit in the tray menu stops everything.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::NewWindowResponse;
use tauri::window::Color;
use tauri::{AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindowBuilder, WindowEvent};
use vbind_service::{Options, ServiceHandle};

/// The in-process engine, or None when the window shows an already running `vbind serve`.
struct Service(Mutex<Option<ServiceHandle>>);

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .setup(|app| {
            let (url, handle) = match vbind_service::start(Options { port: None }) {
                Ok(h) => (h.url(), Some(h)),
                Err(e) => match vbind_service::running_instance() {
                    Some(info) => (format!("http://127.0.0.1:{}/", info.port), None),
                    None => {
                        error_box(&format!("VBind could not start.\n\n{e}"));
                        std::process::exit(1);
                    }
                },
            };
            app.manage(Service(Mutex::new(handle)));
            let home: Url = url.parse()?;
            let origin = home.origin();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(home))
                .title("VBind")
                .inner_size(1480.0, 920.0)
                .min_inner_size(1000.0, 640.0)
                .background_color(Color(20, 18, 17, 255))
                .on_navigation(move |u| {
                    if u.origin() == origin {
                        return true;
                    }
                    open_in_browser(u);
                    false
                })
                .on_new_window(|u, _| {
                    open_in_browser(&u);
                    NewWindowResponse::Deny
                })
                .build()?;

            let open_item = MenuItem::with_id(app, "open", "Open VBind", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit VBind", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open_item, &quit_item])?;
            let mut tray = TrayIconBuilder::with_id("vbind")
                .tooltip("VBind: mappings keep running while this is here")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| match e.id.as_ref() {
                    "open" => show_main(app),
                    "quit" => quit(app),
                    _ => {}
                })
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        show_main(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Games need the virtual controller to stay plugged in, so closing
            // the window only hides it.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build the VBind window");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            stop_service(app);
        }
    });
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn stop_service(app: &AppHandle) {
    if let Some(s) = app.try_state::<Service>() {
        if let Some(h) = s.0.lock().unwrap().take() {
            h.stop();
        }
    }
}

fn quit(app: &AppHandle) {
    stop_service(app);
    app.exit(0);
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Open web links (driver downloads, docs) in the default browser.
fn open_in_browser(u: &Url) {
    if !matches!(u.scheme(), "http" | "https") {
        return;
    }
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::PCWSTR;
    let verb = wide("open");
    let target = wide(u.as_str());
    unsafe {
        ShellExecuteW(None, PCWSTR(verb.as_ptr()), PCWSTR(target.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}

fn error_box(text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::PCWSTR;
    let t = wide(text);
    let c = wide("VBind");
    unsafe {
        MessageBoxW(None, PCWSTR(t.as_ptr()), PCWSTR(c.as_ptr()), MB_OK | MB_ICONERROR);
    }
}
