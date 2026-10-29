use fiend_desktop::{AppView, Assets, Prefs, spawn_backend};
use gpui_kit::prelude::*;
use gpui_kit::{
    Bounds, TitlebarOptions, WindowBounds, WindowDecorations, WindowOptions, point, px, size,
};

fn main() {
    let socket = fiend_ipc::path::default_socket();
    let prefs = Prefs::load();
    gpui_kit::application().with_assets(Assets).run(move |cx| {
        gpui_kit::init(cx);
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
        cx.open_window(options, |window, cx| {
            cx.new(|cx| AppView::new(window, cx, backend, updates, prefs))
        })
        .expect("cannot open the window");
        cx.activate(true);
    });
}
