use gpui_kit::prelude::*;
use gpui_kit::{Div, FontWeight, MouseButton, Window, WindowControlArea, div, px};

use crate::design::Ui;

pub fn caption_bar(bar: Div, ui: &Ui, window: &mut Window) -> Div {
    let p = ui.p;
    let button = |id: &'static str, glyph: &'static str, area: WindowControlArea| {
        div()
            .id(id)
            .w(px(40.))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(15.))
            .font_weight(FontWeight::NORMAL)
            .text_color(p.muted)
            .window_control_area(area)
            .hover(move |s| s.bg(p.hover).text_color(p.text))
            .child(glyph)
    };
    let _ = window;
    bar.window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |event, window, _| {
            if event.click_count == 2 {
                window.zoom_window();
            } else {
                window.start_window_move();
            }
        })
        .child(
            div()
                .absolute()
                .right_0()
                .top_0()
                .h_full()
                .flex()
                .child(button("caption-min", "−", WindowControlArea::Min))
                .child(button("caption-max", "□", WindowControlArea::Max))
                .child(button("caption-close", "×", WindowControlArea::Close)),
        )
        .relative()
}
