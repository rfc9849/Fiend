use gpui_kit::base::{Button, Switch};
use gpui_kit::prelude::*;
use gpui_kit::{AnyElement, Div, ElementId, FontWeight, Rgba, SharedString, Svg, div, px, svg};

use super::theme::{FONT, MONO, Palette};

const SYSTEM_ASCENT_MINUS_DESCENT: f32 = 0.756;

pub fn eid(name: impl Into<String>) -> ElementId {
    ElementId::Name(SharedString::from(name.into()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonKind {
    Default,
    Primary,
    Ghost,
    Danger,
}

#[derive(Clone, Copy)]
pub struct Ui {
    pub p: Palette,
}

impl Ui {
    pub fn new(p: Palette) -> Self {
        Self { p }
    }

    pub fn icon(&self, name: &str, size: f32, color: Rgba) -> Svg {
        svg()
            .path(SharedString::from(format!("icons/{name}.svg")))
            .size(px(size))
            .flex_none()
            .text_color(color)
    }

    pub fn mono(&self, text: impl Into<SharedString>) -> Div {
        div()
            .font_family(MONO)
            .text_size(px(11.))
            .child(text.into())
    }

    pub fn button(
        &self,
        id: impl Into<ElementId>,
        label: &str,
        icon: Option<&str>,
        kind: ButtonKind,
    ) -> Button {
        let p = self.p;
        let (bg, fg, hover, border) = match kind {
            ButtonKind::Default => (p.panel, p.text, p.hover, p.line),
            ButtonKind::Primary => (p.accent, p.on_accent, p.accent_hover, p.accent),
            ButtonKind::Ghost => (p.surface, p.muted, p.hover, p.surface),
            ButtonKind::Danger => (p.panel, p.danger, p.hover, p.line),
        };
        let bg = if kind == ButtonKind::Ghost {
            gpui_kit::transparent_black()
        } else {
            bg.into()
        };
        let border = if kind == ButtonKind::Ghost {
            gpui_kit::transparent_black()
        } else {
            border.into()
        };
        Button::new(id)
            .accessibility_label(label.to_owned())
            .min_h(px(33.))
            .px(px(12.))
            .gap(px(7.))
            .rounded(px(4.))
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_color(fg)
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .focus_visible(move |style| style.border_color(p.accent))
            .when_some(icon, |this, name| this.child(self.icon(name, 15., fg)))
            .child(label.to_owned())
    }

    pub fn compact_button(
        &self,
        id: impl Into<ElementId>,
        label: &str,
        icon: Option<&str>,
        kind: ButtonKind,
    ) -> Button {
        self.button(id, label, icon, kind)
            .min_h(px(28.))
            .px(px(10.))
            .text_size(px(10.))
    }

    pub fn icon_button(&self, id: impl Into<ElementId>, icon: &str, label: &str) -> Button {
        let p = self.p;
        Button::new(id)
            .accessibility_label(label.to_owned())
            .size(px(32.))
            .flex_none()
            .rounded(px(4.))
            .text_color(p.muted)
            .cursor_pointer()
            .hover(move |style| style.bg(p.hover).text_color(p.text))
            .focus_visible(move |style| style.border_1().border_color(p.accent))
            .child(self.icon(icon, 17., p.muted))
    }

    pub fn switch(&self, id: impl Into<ElementId>, checked: bool, label: &str) -> Switch {
        let p = self.p;
        let track = if checked { p.accent } else { p.line };
        let thumb = if checked { p.on_accent } else { p.panel };
        let mut switch = Switch::new(id)
            .checked(checked)
            .accessibility_label(label.to_owned())
            .w(px(30.))
            .h(px(18.))
            .p(px(2.))
            .flex_none()
            .rounded(px(9.))
            .bg(track)
            .cursor_pointer()
            .flex()
            .items_center();
        if checked {
            switch = switch.justify_end();
        }
        switch.child(div().size(px(14.)).rounded_full().bg(thumb))
    }

    pub fn avatar(&self, text: &str, size: f32, font: f32) -> Div {
        let p = self.p;
        div()
            .size(px(size))
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .rounded(px(4.))
            .bg(p.soft)
            .border_1()
            .border_color(p.line)
            .text_color(p.muted)
            .text_size(px(font))
            .font_weight(FontWeight::MEDIUM)
            .child(text.to_owned())
    }

    pub fn tag(&self, text: &str, green: bool) -> Div {
        let p = self.p;
        let base = div()
            .flex()
            .items_center()
            .gap(px(5.))
            .px(px(7.))
            .py(px(3.))
            .rounded(px(3.))
            .text_size(px(10.))
            .child(text.to_owned());
        if green {
            base.bg(p.green_soft).text_color(p.green)
        } else {
            base.bg(p.soft)
                .text_color(p.muted)
                .border_1()
                .border_color(p.line)
        }
    }

    pub fn stat_card(
        &self,
        label: &str,
        icon: &str,
        value: &str,
        unit: &str,
        footer: impl IntoElement,
    ) -> Div {
        let p = self.p;
        div()
            .flex_1()
            .min_w_0()
            .min_h(px(95.))
            .p(px(14.))
            .rounded(px(6.))
            .bg(p.panel)
            .border_1()
            .border_color(p.line)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(11.))
                    .text_color(p.muted)
                    .child(label.to_owned())
                    .child(self.icon(icon, 13., p.muted)),
            )
            .child(self.number_with_unit(value, 22., unit, 10., 5.).mt(px(9.)))
            .child(
                div()
                    .mt(px(10.))
                    .h(px(super::SPARK_HEIGHT))
                    .flex()
                    .items_end()
                    .child(footer),
            )
    }

    pub fn number_with_unit(
        &self,
        value: &str,
        value_size: f32,
        unit: &str,
        unit_size: f32,
        gap: f32,
    ) -> Div {
        let value_line = value_size * 1.4;
        let unit_line = value_line - (value_size - unit_size) * SYSTEM_ASCENT_MINUS_DESCENT;
        div()
            .flex()
            .items_end()
            .whitespace_nowrap()
            .child(
                div()
                    .text_size(px(value_size))
                    .line_height(px(value_line))
                    .font_weight(FontWeight::MEDIUM)
                    .child(value.to_owned()),
            )
            .when(!unit.is_empty(), |this| {
                this.child(
                    div()
                        .ml(px(gap))
                        .text_size(px(unit_size))
                        .line_height(px(unit_line))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(self.p.muted)
                        .child(unit.to_owned()),
                )
            })
    }

    pub fn group(&self) -> Div {
        div()
            .flex()
            .flex_col()
            .bg(self.p.panel)
            .border_1()
            .border_color(self.p.line)
            .rounded(px(5.))
            .overflow_hidden()
    }

    pub fn row(&self, first: bool) -> Div {
        let row = div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(14.))
            .px(px(14.))
            .py(px(11.))
            .min_h(px(43.));
        if first {
            row
        } else {
            row.border_t_1().border_color(self.p.line)
        }
    }

    pub fn info_row(&self, label: &str, value: impl IntoElement, first: bool) -> Div {
        self.row(first)
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(self.p.muted)
                    .child(label.to_owned()),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(4.))
                    .text_size(px(11.))
                    .child(value),
            )
    }

    pub fn setting_row(
        &self,
        title: &str,
        description: Option<&str>,
        control: impl IntoElement,
        first: bool,
    ) -> Div {
        self.row(first)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(title.to_owned()),
                    )
                    .when_some(description, |this, text| {
                        this.child(
                            div()
                                .mt(px(3.))
                                .max_w(px(440.))
                                .text_size(px(11.))
                                .text_color(self.p.muted)
                                .child(text.to_owned()),
                        )
                    }),
            )
            .child(control)
    }

    pub fn section_heading(&self, title: &str, action: Option<AnyElement>) -> Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.))
            .mt(px(24.))
            .mb(px(12.))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title.to_owned()),
            )
            .when_some(action, |this, action| this.child(action))
    }

    pub fn group_title(&self, title: &str) -> Div {
        div()
            .mt(px(22.))
            .mb(px(9.))
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(title.to_owned())
    }

    pub fn help(&self, text: &str) -> Div {
        div()
            .mt(px(9.))
            .mb(px(21.))
            .text_size(px(10.))
            .text_color(self.p.muted)
            .child(text.to_owned())
    }

    pub fn notice(&self, icon: &str, text: &str, danger: bool) -> Div {
        let color = if danger { self.p.danger } else { self.p.text };
        div()
            .flex()
            .gap(px(9.))
            .p(px(12.))
            .mb(px(15.))
            .rounded(px(4.))
            .bg(self.p.soft)
            .text_size(px(11.))
            .text_color(color)
            .child(self.icon(icon, 16., color))
            .child(div().flex_1().min_w_0().child(text.to_owned()))
    }

    pub fn page_title(&self, title: &str) -> Div {
        div()
            .text_size(px(26.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(title.to_owned())
    }

    pub fn field_label(&self, text: &str) -> Div {
        div()
            .mb(px(6.))
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .child(text.to_owned())
    }

    pub fn field_error(&self, text: &str) -> Div {
        div()
            .mt(px(5.))
            .text_size(px(11.))
            .text_color(self.p.danger)
            .child(text.to_owned())
    }

    pub fn font(&self) -> &'static str {
        FONT
    }
}
