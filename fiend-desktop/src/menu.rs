use std::time::{Duration, Instant};

use gpui_kit::base::Button;
use gpui_kit::prelude::*;
use gpui_kit::{
    Anchor, Context, FocusHandle, FontWeight, IntoElement, KeyDownEvent, Window, anchored,
    deferred, div, point, px,
};

use crate::app::{AppView, LogFilter};
use crate::design::Ui;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuId {
    Profile,
    LogLevel,
    Template,
    SubscriptionKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuOption {
    pub value: String,
    pub label: String,
    pub detail: Option<String>,
}

impl MenuOption {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            detail: None,
        }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

pub struct OpenMenu {
    pub id: MenuId,
    pub options: Vec<MenuOption>,
    pub selected: usize,
    pub highlighted: usize,
    pub typed: String,
    pub typed_at: Instant,
}

const PAGE: usize = 8;
const TYPEAHEAD_RESET: Duration = Duration::from_millis(700);

impl OpenMenu {
    pub fn new(id: MenuId, options: Vec<MenuOption>, selected_value: &str) -> Self {
        let selected = options
            .iter()
            .position(|option| option.value == selected_value)
            .unwrap_or(0);
        Self {
            id,
            options,
            selected,
            highlighted: selected,
            typed: String::new(),
            typed_at: Instant::now(),
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        if self.options.is_empty() {
            return;
        }
        let last = self.options.len() as isize - 1;
        self.highlighted = (self.highlighted as isize + delta).clamp(0, last) as usize;
    }

    pub fn first(&mut self) {
        self.highlighted = 0;
    }

    pub fn last(&mut self) {
        self.highlighted = self.options.len().saturating_sub(1);
    }

    pub fn page(&mut self, down: bool) {
        self.move_by(if down {
            PAGE as isize
        } else {
            -(PAGE as isize)
        });
    }

    pub fn type_char(&mut self, text: &str) {
        let now = Instant::now();
        if now.duration_since(self.typed_at) > TYPEAHEAD_RESET {
            self.typed.clear();
        }
        self.typed_at = now;
        self.typed.push_str(&text.to_lowercase());
        let start = if self.typed.chars().count() == 1 {
            self.highlighted + 1
        } else {
            self.highlighted
        };
        let count = self.options.len();
        let found = (0..count)
            .map(|offset| (start + offset) % count)
            .find(|&index| {
                self.options[index]
                    .label
                    .to_lowercase()
                    .starts_with(&self.typed)
            });
        if let Some(index) = found {
            self.highlighted = index;
        }
    }
}

impl AppView {
    pub fn trigger_focus(&mut self, id: MenuId, cx: &mut Context<Self>) -> FocusHandle {
        self.trigger_handles
            .entry(id)
            .or_insert_with(|| cx.focus_handle())
            .clone()
    }

    pub fn open_menu(
        &mut self,
        id: MenuId,
        options: Vec<MenuOption>,
        selected_value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((closed, at)) = self.menu_closed_at
            && closed == id
            && at.elapsed() < Duration::from_millis(200)
        {
            return;
        }
        if options.is_empty() {
            return;
        }
        self.menu = Some(OpenMenu::new(id, options, selected_value));
        let focus = self.menu_focus.clone();
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    pub fn close_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(menu) = self.menu.take() {
            self.menu_closed_at = Some((menu.id, Instant::now()));
            let handle = self.trigger_handles.get(&menu.id).cloned();
            let fallback = self.focus.clone();
            window.defer(cx, move |window, cx| {
                window.focus(&handle.unwrap_or(fallback), cx);
            });
        }
        cx.notify();
    }

    pub fn choose_menu_item(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.as_ref() else {
            return;
        };
        let Some(option) = menu.options.get(index) else {
            return;
        };
        let (id, value) = (menu.id, option.value.clone());
        self.close_menu(window, cx);
        self.apply_menu(id, &value, window, cx);
    }

    fn apply_menu(&mut self, id: MenuId, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        match id {
            MenuId::Profile => self.pick_profile(value.to_owned(), cx),
            MenuId::LogLevel => {
                self.log_filter = LogFilter::from_value(value);
                cx.notify();
            }
            MenuId::Template | MenuId::SubscriptionKind => {
                self.apply_dialog_menu(id, value, window, cx)
            }
        }
    }

    pub fn menu_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(menu) = self.menu.as_mut() else {
            return;
        };
        let key = event.keystroke.key.as_str();
        let modified = event.keystroke.modifiers.control
            || event.keystroke.modifiers.platform
            || event.keystroke.modifiers.alt;
        match key {
            "down" => menu.move_by(1),
            "up" => menu.move_by(-1),
            "home" => menu.first(),
            "end" => menu.last(),
            "pagedown" => menu.page(true),
            "pageup" => menu.page(false),
            "enter" | "space" | "tab" => {
                let index = menu.highlighted;
                self.choose_menu_item(index, window, cx);
                cx.stop_propagation();
                return;
            }
            "escape" => {
                self.close_menu(window, cx);
                cx.stop_propagation();
                return;
            }
            _ if !modified => {
                if let Some(text) = event.keystroke.key_char.as_deref()
                    && !text.trim().is_empty()
                {
                    menu.type_char(text);
                }
            }
            _ => {}
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn menu_popup(
        &self,
        ui: &Ui,
        menu: &OpenMenu,
        width: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = ui.p;
        let id = menu.id;
        let rows = menu.options.iter().enumerate().map(|(index, option)| {
            let highlighted = index == menu.highlighted;
            let selected = index == menu.selected;
            div()
                .id(("menu-option", index))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .min_h(px(32.))
                .rounded(px(3.))
                .cursor_pointer()
                .text_size(px(12.))
                .when(highlighted, |this| this.bg(p.hover))
                .hover(move |s| s.bg(p.hover))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.choose_menu_item(index, window, cx);
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(option.label.clone()),
                        )
                        .when_some(option.detail.clone(), |this, detail| {
                            this.child(div().text_size(px(10.)).text_color(p.muted).child(detail))
                        }),
                )
                .when(selected, |this| this.child(ui.icon("check", 13., p.accent)))
        });
        let _ = id;
        div()
            .id("menu-popup")
            .track_focus(&self.menu_focus)
            .key_context("Menu")
            .on_key_down(
                cx.listener(|this, event, window, cx| this.menu_key_down(event, window, cx)),
            )
            .on_mouse_down_out(cx.listener(|this, _, window, cx| this.close_menu(window, cx)))
            .occlude()
            .w(px(width))
            .max_h(px(280.))
            .overflow_y_scroll()
            .p(px(4.))
            .rounded(px(5.))
            .bg(p.panel)
            .border_1()
            .border_color(p.line)
            .shadow_lg()
            .flex()
            .flex_col()
            .children(rows)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn select(
        &mut self,
        ui: &Ui,
        id: MenuId,
        label: String,
        options: Vec<MenuOption>,
        selected_value: String,
        inline: bool,
        width: f32,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let p = ui.p;
        let focus = self.trigger_focus(id, cx);
        let open = self.menu.as_ref().is_some_and(|menu| menu.id == id);
        let height = if inline { 27. } else { 36. };
        let opts = options.clone();
        let current = selected_value.clone();
        let trigger = Button::new(crate::design::eid(format!("select-{id:?}")))
            .accessibility_label(label.clone())
            .track_focus(&focus)
            .when(!inline, |this| this.w_full())
            .min_h(px(height))
            .gap(px(if inline { 6. } else { 12. }))
            .justify_between()
            .text_size(px(if inline { 14. } else { 12. }))
            .font_weight(FontWeight::NORMAL)
            .cursor_pointer()
            .when(!inline, |this| {
                this.px(px(10.))
                    .py(px(8.))
                    .rounded(px(4.))
                    .bg(p.panel)
                    .border_1()
                    .border_color(if open { p.accent } else { p.line })
                    .hover(move |s| s.bg(p.hover))
            })
            .when(inline, |this| this.hover(move |s| s.text_color(p.accent)))
            .focus_visible(move |s| s.border_1().border_color(p.accent))
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.menu.as_ref().is_some_and(|menu| menu.id == id) {
                    this.close_menu(window, cx);
                } else {
                    this.open_menu(id, opts.clone(), &current, window, cx);
                }
            }))
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .child(label),
            )
            .child(
                ui.icon("chevron", 13., p.muted)
                    .when(open, |s| s.opacity(0.7)),
            );
        let popup = self
            .menu
            .as_ref()
            .filter(|menu| menu.id == id)
            .map(|menu| self.menu_popup(ui, menu, width.max(180.), cx));
        div()
            .relative()
            .when(!inline, |this| this.w(px(width)))
            .child(trigger)
            .when_some(popup, |this, popup| {
                this.child(deferred(
                    anchored()
                        .anchor(Anchor::TopLeft)
                        .offset(point(px(0.), px(height + 4.)))
                        .snap_to_window_with_margin(px(8.))
                        .child(popup),
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu() -> OpenMenu {
        OpenMenu::new(
            MenuId::LogLevel,
            vec![
                MenuOption::new("all", "All levels"),
                MenuOption::new("info", "Info"),
                MenuOption::new("debug", "Debug"),
                MenuOption::new("warn", "Warnings"),
            ],
            "info",
        )
    }

    #[test]
    fn selection_starts_at_the_current_value() {
        let menu = menu();
        assert_eq!((menu.selected, menu.highlighted), (1, 1));
        assert_eq!(
            OpenMenu::new(MenuId::Profile, menu.options.clone(), "none").highlighted,
            0
        );
    }

    #[test]
    fn movement_stops_at_the_ends() {
        let mut menu = menu();
        menu.move_by(-5);
        assert_eq!(menu.highlighted, 0);
        menu.move_by(10);
        assert_eq!(menu.highlighted, 3);
        menu.first();
        assert_eq!(menu.highlighted, 0);
        menu.last();
        assert_eq!(menu.highlighted, 3);
        menu.page(false);
        assert_eq!(menu.highlighted, 0);
        menu.page(true);
        assert_eq!(menu.highlighted, 3);
    }

    #[test]
    fn typing_finds_the_next_match_and_builds_a_prefix() {
        let mut menu = menu();
        menu.type_char("d");
        assert_eq!(menu.highlighted, 2);
        menu.typed_at = Instant::now() - Duration::from_secs(2);
        menu.type_char("w");
        assert_eq!(menu.highlighted, 3);
        menu.typed_at = Instant::now() - Duration::from_secs(2);
        menu.type_char("a");
        menu.type_char("l");
        assert_eq!(menu.highlighted, 0);
    }
}
