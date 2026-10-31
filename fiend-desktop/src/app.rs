use std::time::Duration;

use fiend_ipc::{LogLevel, Reply, Request, TunnelState};
use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver;
use gpui_kit::prelude::*;
use gpui_kit::{Context, FocusHandle, FontWeight, IntoElement, Render, Window, div, px};

use crate::client::Backend;
use crate::design::{FONT, Palette, ThemeMode, Ui};
use crate::dialogs::Dialog;
use crate::format;
use crate::menu::OpenMenu;
use crate::model::{Model, Update};
use crate::prefs::Prefs;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Home,
    Profiles,
    Routing,
    Logs,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsTab {
    General,
    Appearance,
    About,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogFilter {
    All,
    Info,
    Debug,
    Warn,
}

impl LogFilter {
    pub fn matches(self, level: LogLevel) -> bool {
        match self {
            Self::All => true,
            Self::Info => level == LogLevel::Info,
            Self::Debug => matches!(level, LogLevel::Debug | LogLevel::Trace),
            Self::Warn => matches!(level, LogLevel::Warn | LogLevel::Error),
        }
    }

    pub fn value(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Warn => "warn",
        }
    }

    pub fn from_value(value: &str) -> Self {
        match value {
            "info" => Self::Info,
            "debug" => Self::Debug,
            "warn" => Self::Warn,
            _ => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All levels",
            Self::Info => "Info",
            Self::Debug => "Debug",
            Self::Warn => "Warnings",
        }
    }
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub id: u64,
}

pub struct AppView {
    pub model: Model,
    pub backend: Backend,
    pub prefs: Prefs,
    pub page: Page,
    pub settings_tab: SettingsTab,
    pub selected_profile: Option<String>,
    pub log_filter: LogFilter,
    pub logs_paused: bool,
    pub frozen_logs: Option<Vec<fiend_ipc::LogEntry>>,
    pub dialog: Option<Dialog>,
    pub suspended_dialog: Option<Dialog>,
    pub menu: Option<OpenMenu>,
    pub menu_closed_at: Option<(crate::menu::MenuId, std::time::Instant)>,
    pub trigger_handles: std::collections::HashMap<crate::menu::MenuId, FocusHandle>,
    pub toast: Option<Toast>,
    pub toast_counter: u64,
    pub busy: bool,
    pub focus: FocusHandle,
    pub menu_focus: FocusHandle,
    pub dialog_focus: FocusHandle,
    pub now_ms: u64,
    pub service_outdated: bool,
    pub pending_restore: Option<crate::backup::BackupFile>,
}

impl AppView {
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        backend: Backend,
        mut updates: UnboundedReceiver<Update>,
        prefs: Prefs,
    ) -> Self {
        cx.spawn(async move |this, cx| {
            while let Some(update) = updates.next().await {
                if this
                    .update(cx, |this, cx| this.on_update(update, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let alive = this.update(cx, |this, cx| {
                    this.now_ms = format::now_ms();
                    if matches!(this.model.status.state, TunnelState::Connected { .. })
                        && this.page == Page::Home
                    {
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
        cx.observe_window_appearance(window, |_, _, cx| cx.notify())
            .detach();
        let focus = cx.focus_handle();
        Self {
            model: Model::default(),
            backend,
            prefs,
            page: Page::Home,
            settings_tab: SettingsTab::General,
            selected_profile: None,
            log_filter: LogFilter::All,
            logs_paused: false,
            frozen_logs: None,
            dialog: None,
            suspended_dialog: None,
            menu: None,
            menu_closed_at: None,
            trigger_handles: std::collections::HashMap::new(),
            toast: None,
            toast_counter: 0,
            busy: false,
            focus,
            menu_focus: cx.focus_handle(),
            dialog_focus: cx.focus_handle(),
            now_ms: format::now_ms(),
            service_outdated: crate::service::needs_update(),
            pending_restore: None,
        }
    }

    pub fn palette(&self, window: &Window) -> Palette {
        Palette::of(self.prefs.theme.resolve(window.appearance()))
    }

    pub fn on_update(&mut self, update: Update, cx: &mut Context<Self>) {
        let was_ready = self.model.service.is_ready();
        if let Update::Profiles(profiles) = &update {
            let valid = self
                .selected_profile
                .as_ref()
                .is_some_and(|id| profiles.iter().any(|profile| &profile.id == id));
            if !valid {
                self.selected_profile = profiles.first().map(|profile| profile.id.clone());
            }
        }
        self.model.apply(update);
        if !was_ready && self.model.service.is_ready() {
            self.after_service_ready(cx);
        }
        cx.notify();
    }

    fn after_service_ready(&mut self, cx: &mut Context<Self>) {
        if self.service_outdated && self.dialog.is_none() {
            self.dialog = Some(Dialog::Service(crate::dialogs::ServiceDialog {
                mode: crate::dialogs::ServiceMode::Update,
                busy: false,
                error: None,
            }));
        }
        if self.selected_profile.is_none() {
            self.selected_profile = self
                .model
                .settings
                .active_profile
                .clone()
                .or_else(|| self.model.profiles.first().map(|p| p.id.clone()));
        }
        static OPENED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if self.prefs.connect_on_open
            && !OPENED.swap(true, std::sync::atomic::Ordering::SeqCst)
            && matches!(self.model.status.state, TunnelState::Disconnected)
            && (self.model.settings.active_profile.is_some() || !self.model.profiles.is_empty())
        {
            self.connect(None, cx);
        }
    }

    pub fn show_toast(&mut self, text: impl Into<String>, error: bool, cx: &mut Context<Self>) {
        self.toast_counter += 1;
        let id = self.toast_counter;
        self.toast = Some(Toast {
            text: text.into(),
            error,
            id,
        });
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(3200))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.toast.as_ref().is_some_and(|toast| toast.id == id) {
                    this.toast = None;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }

    pub fn call(
        &mut self,
        request: Request,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, Result<Reply, String>, &mut Context<Self>) + 'static,
    ) {
        let receiver = self.backend.call(request);
        cx.spawn(async move |this, cx| {
            let result = receiver
                .await
                .unwrap_or_else(|_| Err("The service closed the connection".into()));
            let _ = this.update(cx, |this, cx| {
                done(this, result, cx);
                cx.notify();
            });
        })
        .detach();
    }

    pub fn call_toast(&mut self, request: Request, cx: &mut Context<Self>) {
        self.call(request, cx, |this, result, cx| {
            if let Err(error) = result {
                this.show_toast(error, true, cx);
            }
        });
    }

    pub fn connect(&mut self, profile_id: Option<String>, cx: &mut Context<Self>) {
        let id = profile_id
            .or_else(|| self.model.settings.active_profile.clone())
            .or_else(|| self.model.profiles.first().map(|p| p.id.clone()));
        self.call_toast(Request::Connect { profile_id: id }, cx);
    }

    pub fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.call_toast(Request::Disconnect, cx);
    }

    pub fn toggle_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.model.service.is_ready() {
            self.open_service_dialog(window, cx);
            return;
        }
        if self.model.profiles.is_empty() {
            self.open_profile_form(None, window, cx);
            return;
        }
        match self.model.status.state {
            TunnelState::Connected { .. } | TunnelState::Connecting { .. } => self.disconnect(cx),
            TunnelState::Disconnecting => {}
            TunnelState::Disconnected | TunnelState::Failed { .. } => self.connect(None, cx),
        }
    }

    pub fn navigate(&mut self, page: Page, cx: &mut Context<Self>) {
        self.page = page;
        self.menu = None;
        cx.notify();
    }

    pub fn set_theme(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        self.prefs.theme = mode;
        self.prefs.save();
        cx.notify();
    }

    fn nav_item(
        &self,
        ui: &Ui,
        page: Page,
        icon: &'static str,
        label: &'static str,
        count: Option<usize>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let p = ui.p;
        let active = self.page == page;
        gpui_kit::base::Button::new(crate::design::eid(format!("{}-{}", "nav", label)))
            .accessibility_label(label)
            .w_full()
            .h(px(39.))
            .px(px(12.))
            .gap(px(10.))
            .justify_start()
            .rounded(px(4.))
            .text_size(px(12.))
            .cursor_pointer()
            .text_color(if active { p.text } else { p.muted })
            .when(active, |this| {
                this.bg(p.panel)
                    .border_1()
                    .border_color(p.line)
                    .font_weight(FontWeight::MEDIUM)
            })
            .when(!active, |this| {
                this.hover(move |s| s.bg(p.hover).text_color(p.text))
            })
            .focus_visible(move |s| s.border_1().border_color(p.accent))
            .on_click(cx.listener(move |this, _, _, cx| this.navigate(page, cx)))
            .child(ui.icon(icon, 17., if active { p.text } else { p.muted }))
            .child(div().child(label))
            .when_some(count, |this, count| {
                this.child(
                    div()
                        .ml_auto()
                        .text_size(px(10.))
                        .text_color(p.muted)
                        .child(count.to_string()),
                )
            })
    }

    fn sidebar(&self, ui: &Ui, cx: &mut Context<Self>) -> impl IntoElement {
        let p = ui.p;
        div()
            .w(px(174.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .bg(p.sidebar)
            .px(px(10.))
            .pt(px(6.))
            .pb(px(18.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .px(px(12.))
                    .pt(px(5.))
                    .pb(px(30.))
                    .child(
                        div()
                            .relative()
                            .size(px(28.))
                            .child(ui.icon("brand", 28., p.text).absolute().top_0().left_0())
                            .child(
                                ui.icon("brand-face", 28., p.sidebar)
                                    .absolute()
                                    .top_0()
                                    .left_0(),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(29.))
                            .line_height(px(29.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("fiend"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(self.nav_item(ui, Page::Home, "home", "Connection", None, cx))
                    .child(self.nav_item(
                        ui,
                        Page::Profiles,
                        "server",
                        "Profiles",
                        Some(self.model.profiles.len()),
                        cx,
                    ))
                    .child(self.nav_item(ui, Page::Routing, "route", "Routing", None, cx))
                    .child(self.nav_item(ui, Page::Logs, "log", "Logs", None, cx)),
            )
            .child(div().mt_auto().child(self.nav_item(
                ui,
                Page::Settings,
                "settings",
                "Settings",
                None,
                cx,
            )))
    }

    fn window_bar(&self, ui: &Ui, window: &mut Window) -> impl IntoElement {
        let p = ui.p;
        let bar = div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(44.))
            .w_full()
            .child(div().w(px(174.)).h_full().flex_none().bg(p.sidebar))
            .child(div().flex_1().h_full().bg(p.surface));
        #[cfg(target_os = "macos")]
        {
            let _ = window;
            bar.window_control_area(gpui_kit::WindowControlArea::Drag)
        }
        #[cfg(not(target_os = "macos"))]
        {
            crate::chrome::caption_bar(bar, ui, window)
        }
    }

    pub fn handle_message(
        &mut self,
        message: crate::single::Message,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.activate_window();
        if let crate::single::Message::Open(link) = message {
            self.page = Page::Profiles;
            self.open_import(Some(link), window, cx);
        }
        cx.notify();
    }

    pub fn is_ready(&self) -> bool {
        self.model.service.is_ready()
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = self.palette(window);
        let ui = Ui::new(p);
        let page = match self.page {
            Page::Home => self.page_home(&ui, window, cx).into_any_element(),
            Page::Profiles => self.page_profiles(&ui, window, cx).into_any_element(),
            Page::Routing => self.page_routing(&ui, window, cx).into_any_element(),
            Page::Logs => self.page_logs(&ui, window, cx).into_any_element(),
            Page::Settings => self.page_settings(&ui, window, cx).into_any_element(),
        };
        let dialog = self.render_dialog(&ui, window, cx);
        let toast = self.toast.as_ref().map(|toast| {
            div()
                .absolute()
                .bottom(px(25.))
                .w_full()
                .flex()
                .justify_center()
                .child(
                    div()
                        .max_w(px(560.))
                        .px(px(18.))
                        .py(px(11.))
                        .rounded(px(5.))
                        .bg(if toast.error { p.danger } else { p.text })
                        .text_color(if toast.error { p.on_accent } else { p.surface })
                        .text_size(px(12.))
                        .child(toast.text.clone()),
                )
        });
        div()
            .id("fiend-root")
            .key_context("Fiend")
            .track_focus(&self.focus)
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(p.surface)
            .text_color(p.text)
            .text_size(px(13.))
            .font_family(FONT)
            .child(self.window_bar(&ui, window))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.sidebar(&ui, cx))
                    .child(
                        div()
                            .id("main")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_y_scroll()
                            .px(px(30.))
                            .pt(px(16.))
                            .pb(px(28.))
                            .child(page),
                    ),
            )
            .when_some(dialog, |this, dialog| this.child(dialog))
            .when_some(toast, |this, toast| this.child(toast))
    }
}
