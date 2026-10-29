#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

use fiend_desktop::single::{Message, Role, acquire};
use fiend_desktop::{AppView, Assets, Prefs, spawn_backend};
use futures::StreamExt;
use gpui_kit::prelude::*;
use gpui_kit::{
    Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowBounds, WindowDecorations,
    WindowOptions, point, px, size,
};

gpui_kit::actions!([Quit]);

#[cfg(windows)]
fn register_protocol() {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let Some(program) = std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .map(|root| root.join(r"System32\reg.exe"))
    else {
        return;
    };
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let key = r"HKCU\Software\Classes\fiend";
    let open = format!(r"{}\shell\open\command", key);
    let command = format!("\"{}\" \"%1\"", exe.display());
    let entries = [
        vec![key, "/ve", "/d", "URL:Fiend Protocol", "/f"],
        vec![key, "/v", "URL Protocol", "/t", "REG_SZ", "/f"],
        vec![open.as_str(), "/ve", "/d", command.as_str(), "/f"],
    ];
    for entry in entries {
        let mut process = std::process::Command::new(&program);
        process
            .arg("add")
            .args(entry)
            .creation_flags(CREATE_NO_WINDOW);
        let _ = process.output();
    }
}

fn main() {
    let first = std::env::args()
        .skip(1)
        .find(|argument| argument.starts_with("fiend://"))
        .map(Message::Open)
        .unwrap_or(Message::Show);
    let lock = fiend_desktop::config_dir()
        .map(|dir| dir.join("desktop.sock"))
        .unwrap_or_else(|| std::env::temp_dir().join("fiend-desktop.sock"));
    let Role::Primary(mut messages) = acquire(&lock, &first) else {
        return;
    };
    #[cfg(windows)]
    register_protocol();
    let socket = fiend_ipc::path::default_socket();
    let prefs = Prefs::load();
    let (url_sender, mut urls) = futures::channel::mpsc::unbounded::<Message>();
    let application = gpui_kit::application().with_assets(Assets);
    application.on_open_urls(move |opened| {
        for url in opened {
            let _ = url_sender.unbounded_send(Message::Open(url));
        }
    });
    application.run(move |cx| {
        gpui_kit::init(cx);
        #[cfg(not(windows))]
        cx.register_url_scheme("fiend").detach();
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("ctrl-q", Quit, None),
        ]);
        cx.set_menus([Menu::new("Fiend").items([MenuItem::action("Quit Fiend", Quit)])]);
        let (backend, updates) = spawn_backend(socket);
        let bounds = Bounds::centered(None, size(px(1000.), px(690.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Fiend".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(15.), px(14.))),
            }),
            window_min_size: Some(size(px(720.), px(520.))),
            window_decorations: Some(WindowDecorations::Client),
            ..Default::default()
        };
        let window = cx
            .open_window(options, |window, cx| {
                cx.new(|cx| AppView::new(window, cx, backend, updates, prefs))
            })
            .expect("cannot open the window");
        cx.spawn(async move |cx| {
            loop {
                let message = futures::select_biased! {
                    message = messages.next() => message,
                    message = urls.next() => message,
                };
                let Some(message) = message else { break };
                let _ = window.update(cx, |view, window, cx| {
                    view.handle_message(message, window, cx)
                });
            }
        })
        .detach();
        cx.activate(true);
    });
}
