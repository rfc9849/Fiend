use fiend_desktop::single::{Message, Role, acquire};
use fiend_desktop::{AppView, Assets, Prefs, spawn_backend};
use futures::StreamExt;
use gpui_kit::prelude::*;
use gpui_kit::{
    Bounds, TitlebarOptions, WindowBounds, WindowDecorations, WindowOptions, point, px, size,
};

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
        cx.register_url_scheme("fiend").detach();
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
