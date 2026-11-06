use std::time::Duration;

use fiend_ipc::{ConnectStep, LogLevel, Request, RuleKind, TunnelState};
use gpui_kit::base::Button;
use gpui_kit::prelude::*;
use gpui_kit::{
    Animation, AnimationExt as _, ClipboardItem, Context, FontWeight, IntoElement, Transformation,
    Window, div, percentage, px,
};

use crate::app::{AppView, LogFilter, SettingsTab};
use crate::design::sparkline;
use crate::design::{ButtonKind, MONO, ThemeMode, Ui};
use crate::dialogs::{ConfirmAction, ValueKind, kind_label, kind_value, rule_count};
use crate::format;
use crate::menu::{MenuId, MenuOption};
use crate::model::{HISTORY_LIMIT, Sample, ServiceState};

const RATE_FLOOR: f32 = 12_500.;
const LATENCY_FLOOR: f32 = 50.;

fn step_text(step: ConnectStep) -> &'static str {
    match step {
        ConnectStep::Server => "Reaching the server",
        ConnectStep::Routes => "Applying routes",
        ConnectStep::Dns => "Applying DNS",
    }
}

impl AppView {
    fn page_header(
        &self,
        ui: &Ui,
        title: &str,
        actions: Vec<gpui_kit::AnyElement>,
    ) -> gpui_kit::Div {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(15.))
            .mb(px(28.))
            .child(ui.page_title(title))
            .child(div().flex().items_center().gap(px(8.)).children(actions))
    }

    pub fn page_home(
        &mut self,
        ui: &Ui,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let p = ui.p;
        let state = self.model.status.state.clone();
        let ready = self.model.service.is_ready();
        let no_profiles = ready && self.model.profiles.is_empty();
        let connected = matches!(state, TunnelState::Connected { .. });
        let (title, detail, action): (String, Option<String>, &str) = if !ready {
            let (title, detail) = match &self.model.service {
                ServiceState::Starting => ("Starting…", String::new()),
                ServiceState::NotInstalled => (
                    "Service not installed",
                    "Administrator access required.".into(),
                ),
                ServiceState::NotRunning => {
                    ("Service not running", "Start the Fiend service.".into())
                }
                ServiceState::Refused(message) => ("Access denied", message.clone()),
                ServiceState::Incompatible(message) => ("Service not supported", message.clone()),
                ServiceState::Ready => ("", String::new()),
            };
            let action = match self.model.service {
                ServiceState::NotInstalled => "Install service",
                ServiceState::NotRunning => "Start service",
                _ => "Service unavailable",
            };
            (
                title.to_owned(),
                Some(detail).filter(|d| !d.is_empty()),
                action,
            )
        } else if no_profiles {
            ("Add your first profile".into(), None, "Add profile")
        } else {
            match &state {
                TunnelState::Disconnected => ("Disconnected".into(), None, "Connect"),
                TunnelState::Connecting { step } => (
                    "Connecting…".into(),
                    Some(step_text(*step).into()),
                    "Cancel connection",
                ),
                TunnelState::Connected { since_ms } => (
                    "Connected".into(),
                    Some(format::elapsed(*since_ms, self.now_ms)),
                    "Disconnect",
                ),
                TunnelState::Disconnecting => (
                    "Disconnecting…".into(),
                    Some("Restoring routes and DNS".into()),
                    "Disconnecting",
                ),
                TunnelState::Failed { reason } => {
                    ("Connection failed".into(), Some(reason.clone()), "Connect")
                }
            }
        };
        let failed = matches!(state, TunnelState::Failed { .. }) && ready;
        let working = matches!(
            state,
            TunnelState::Connecting { .. } | TunnelState::Disconnecting
        ) && ready;
        let ring = if failed {
            p.danger
        } else if connected {
            p.green
        } else {
            p.dial_edge
        };
        let glyph = if failed {
            p.danger
        } else if connected {
            p.green
        } else {
            p.muted
        };
        let disabled = matches!(state, TunnelState::Disconnecting)
            || matches!(self.model.service, ServiceState::Starting);

        let reduce_motion = cx.reduce_motion();
        let power = Button::new("power")
            .accessibility_label(action)
            .disabled(disabled)
            .relative()
            .size(px(160.))
            .flex_none()
            .rounded_full()
            .cursor_pointer()
            .on_click(cx.listener(|this, _, window, cx| this.toggle_connection(window, cx)))
            .child(
                div()
                    .absolute()
                    .top(px(9.))
                    .left(px(9.))
                    .size(px(142.))
                    .rounded_full()
                    .border_1()
                    .border_color(ring),
            )
            .when(working, |this| {
                let arc = ui
                    .icon("arc", 142., p.accent)
                    .absolute()
                    .top(px(9.))
                    .left(px(9.));
                if reduce_motion {
                    this.child(arc)
                } else {
                    this.child(arc.with_animation(
                        "spin",
                        Animation::new(Duration::from_millis(1200)).repeat(),
                        |icon, delta| {
                            icon.with_transformation(Transformation::rotate(percentage(delta)))
                        },
                    ))
                }
            })
            .child(
                div()
                    .absolute()
                    .top(px(17.))
                    .left(px(17.))
                    .size(px(126.))
                    .rounded_full()
                    .bg(p.dial_surface)
                    .border_1()
                    .border_color(p.line)
                    .hover(move |s| s.bg(p.hover)),
            )
            .child(ui.icon("power", 46., glyph));

        let panel = div()
            .min_h(px(270.))
            .p(px(36.))
            .flex()
            .items_center()
            .gap(px(38.))
            .rounded(px(7.))
            .bg(p.hero)
            .border_1()
            .border_color(if failed { p.danger } else { p.line })
            .child(power)
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .child(
                        div()
                            .text_size(px(30.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if failed { p.danger } else { p.text })
                            .child(title),
                    )
                    .when_some(detail, |this, detail| {
                        this.child(
                            div()
                                .mt(px(12.))
                                .font_family(MONO)
                                .text_size(px(12.))
                                .text_color(p.muted)
                                .child(detail),
                        )
                    }),
            );

        let profile = self.model.active_profile().cloned();
        let selector = profile.filter(|_| ready).map(|profile| {
            let options: Vec<MenuOption> = self
                .model
                .profiles
                .iter()
                .map(|item| {
                    MenuOption::new(item.id.clone(), item.name.clone()).detail(item.server_list())
                })
                .collect();
            let select = self.select(
                ui,
                MenuId::Profile,
                profile.name.clone(),
                options,
                profile.id.clone(),
                true,
                300.,
                cx,
            );
            div()
                .flex()
                .items_center()
                .gap(px(14.))
                .px(px(8.))
                .py(px(12.))
                .mb(px(16.))
                .child(ui.avatar(&format::initials(&profile.name), 42., 12.))
                .child(
                    div().flex().flex_col().items_start().child(select).child(
                        div()
                            .mt(px(2.))
                            .font_family(MONO)
                            .text_size(px(10.))
                            .text_color(p.muted)
                            .child(profile.server_list()),
                    ),
                )
        });

        let stats = self.model.status.stats;
        let live = connected && stats.is_some();
        let stats = stats.unwrap_or_default();
        let (down, down_unit) = format::rate(stats.download_bps);
        let (up, up_unit) = format::rate(stats.upload_bps);
        let (total, total_unit) = format::size(stats.sent_bytes + stats.received_bytes);
        let latency = stats
            .latency_ms
            .map(|ms| ms.to_string())
            .unwrap_or_else(|| "—".into());
        let history = &self.model.history;
        let series = |pick: fn(&Sample) -> Option<f32>, floor: f32| {
            let values: Vec<Option<f32>> = history.iter().map(pick).collect();
            sparkline(values, HISTORY_LIMIT, floor, p.accent).into_any_element()
        };
        let (got, got_unit) = format::size(stats.received_bytes);
        let (sent, sent_unit) = format::size(stats.sent_bytes);
        let part = |icon: &str, value: String| {
            div()
                .flex()
                .items_center()
                .gap(px(3.))
                .min_w_0()
                .whitespace_nowrap()
                .child(ui.icon(icon, 10., p.muted))
                .child(value)
        };
        let split = div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(6.))
            .text_size(px(11.))
            .text_color(p.muted)
            .child(part("down", format!("{got} {got_unit}")))
            .child(part("up", format!("{sent} {sent_unit}")))
            .into_any_element();
        let cell = |label: &str, icon: &str, value: String, unit: &str, footer| {
            if live {
                ui.stat_card(label, icon, &value, unit, footer)
            } else {
                ui.stat_card(label, icon, "—", "", div().into_any_element())
            }
        };
        let grid = div()
            .flex()
            .gap(px(10.))
            .child(cell(
                "Download",
                "down",
                down,
                down_unit,
                series(|s| Some(s.download_bps as f32), RATE_FLOOR),
            ))
            .child(cell(
                "Upload",
                "up",
                up,
                up_unit,
                series(|s| Some(s.upload_bps as f32), RATE_FLOOR),
            ))
            .child(cell(
                "Latency",
                "speed",
                latency,
                "ms",
                series(|s| s.latency_ms.map(|ms| ms as f32), LATENCY_FLOOR),
            ))
            .child(cell("Transferred", "data", total, total_unit, split));
        let _ = window;
        div()
            .max_w(px(640.))
            .mx_auto()
            .mt(px(32.))
            .child(panel)
            .when(selector.is_none(), |this| this.child(div().h(px(16.))))
            .when_some(selector, |this, selector| this.child(selector))
            .child(grid)
    }

    pub fn page_profiles(
        &mut self,
        ui: &Ui,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let p = ui.p;
        let actions = vec![
            ui.button("import", "Import", Some("plus"), ButtonKind::Default)
                .on_click(cx.listener(|this, _, window, cx| this.open_import(None, window, cx)))
                .into_any_element(),
            ui.button("new-profile", "New profile", None, ButtonKind::Primary)
                .on_click(
                    cx.listener(|this, _, window, cx| this.open_profile_form(None, window, cx)),
                )
                .into_any_element(),
        ];
        let header = self.page_header(ui, "Profiles", actions);
        if self.model.profiles.is_empty() {
            return div().child(header).child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(16.))
                    .py(px(50.))
                    .min_h(px(230.))
                    .child(ui.icon("server", 32., p.muted))
                    .child(
                        div()
                            .text_size(px(17.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("No profiles yet"),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(p.muted)
                            .child("Create a profile or import a fiend:// link."),
                    )
                    .child(
                        ui.button("empty-new", "New profile", None, ButtonKind::Primary)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_profile_form(None, window, cx)
                            })),
                    ),
            );
        }
        let selected = self
            .selected_profile
            .clone()
            .filter(|id| self.model.profile(id).is_some())
            .or_else(|| {
                self.model
                    .profiles
                    .first()
                    .map(|profile| profile.id.clone())
            });
        let rows = self.model.profiles.iter().map(|profile| {
            let id = profile.id.clone();
            let active = selected.as_deref() == Some(profile.id.as_str());
            let latency = self
                .model
                .latencies
                .get(&profile.id)
                .map(|ms| format!("{ms} ms"))
                .unwrap_or_else(|| "— ms".into());
            Button::new(crate::design::eid(format!("profile-row-{}", profile.id)))
                .accessibility_label(profile.name.clone())
                .w_full()
                .min_h(px(62.))
                .px(px(8.))
                .py(px(10.))
                .gap(px(10.))
                .mb(px(4.))
                .justify_start()
                .rounded(px(4.))
                .cursor_pointer()
                .when(active, |this| this.bg(p.hover))
                .hover(move |s| s.bg(p.hover))
                .focus_visible(move |s| s.border_1().border_color(p.accent))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.selected_profile = Some(id.clone());
                    cx.notify();
                }))
                .child(ui.avatar(&format::initials(&profile.name), 34., 10.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(profile.name.clone()),
                        )
                        .child(
                            div()
                                .mt(px(3.))
                                .text_size(px(10.))
                                .text_color(p.muted)
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(profile.server_list()),
                        ),
                )
                .child(div().text_size(px(10.)).text_color(p.muted).child(latency))
        });
        let list = div()
            .flex_1()
            .min_w(px(180.))
            .pr(px(18.))
            .border_r_1()
            .border_color(p.line)
            .child(div().children(rows))
            .child(
                div()
                    .mt(px(15.))
                    .pt(px(10.))
                    .border_t_1()
                    .border_color(p.line)
                    .child(
                        ui.button("test-all", "Test all", Some("refresh"), ButtonKind::Ghost)
                            .on_click(cx.listener(|this, _, _, cx| this.test_all(cx))),
                    ),
            );
        let detail = selected.as_deref().and_then(|id| self.model.profile(id)).cloned().map(|profile| {
            let id = profile.id.clone();
            let is_active = self.model.status.profile_id.as_deref() == Some(id.as_str());
            let connected = matches!(self.model.status.state, TunnelState::Connected { .. } | TunnelState::Connecting { .. });
            let running_here = is_active && connected;
            let first = profile.servers.first().cloned().unwrap_or_default();
            let (host, port) = format::host_port(&first);
            let more_servers = profile.servers.iter().skip(1).cloned().collect::<Vec<_>>().join(", ");
            let secret_copy = profile.secret.clone();
            let template_name = crate::dialogs::template_label(&profile.template_json);
            let t = profile.tuning;
            let ids = (id.clone(), id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .mt(px(3.))
                        .mb(px(20.))
                        .child(ui.avatar(&format::initials(&profile.name), 52., 15.))
                        .child(div().text_size(px(23.)).font_weight(FontWeight::MEDIUM).child(profile.name.clone())),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .mb(px(20.))
                        .child(
                            ui.compact_button("connect-profile", if running_here { "Disconnect" } else { "Connect" }, Some("power"), ButtonKind::Primary)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if !this.is_ready() {
                                        this.open_service_dialog(window, cx);
                                    } else if running_here {
                                        this.disconnect(cx);
                                    } else {
                                        this.connect(Some(ids.0.clone()), cx);
                                    }
                                })),
                        )
                        .child(
                            ui.compact_button("test-profile", "Test", Some("speed"), ButtonKind::Default)
                                .on_click(cx.listener(move |this, _, _, cx| this.test_profile(ids.1.clone(), cx))),
                        )
                        .child(
                            ui.compact_button("share-profile", "Share", Some("share"), ButtonKind::Default)
                                .on_click(cx.listener(move |this, _, window, cx| this.open_share(&ids.2, window, cx))),
                        )
                        .child(
                            ui.compact_button("edit-profile", "Edit", Some("edit"), ButtonKind::Default)
                                .on_click(cx.listener(move |this, _, window, cx| this.open_profile_form(Some(ids.3.clone()), window, cx))),
                        ),
                )
                .child(ui.group_title("Server"))
                .child(
                    ui.group()
                        .child(ui.info_row("Host", ui.mono(host), true))
                        .child(ui.info_row("Port", ui.mono(port), !more_servers.is_empty()))
                        .when(!more_servers.is_empty(), |list| list.child(ui.info_row("More servers", ui.mono(more_servers), false)))
                        .child(ui.info_row(
                            "Secret",
                            div()
                                .flex()
                                .items_center()
                                .child(
                                    ui.icon_button("secret-copy", "copy", "Copy secret")
                                        .size(px(24.))
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(secret_copy.clone()));
                                            this.show_toast("Secret copied", false, cx);
                                        })),
                                ),
                            false,
                        ))
                        .child(ui.info_row("TLS fingerprint", div().child(template_name), false)),
                )
                .child(ui.group_title("Connection tuning"))
                .child(
                    ui.group()
                        .child(ui.info_row("Handshake timeout", div().child(format!("{} ms", t.handshake_timeout_ms)), true))
                        .child(ui.info_row("Max connections", div().child(t.max_connections.to_string()), false))
                        .child(ui.info_row("UDP idle timeout", div().child(format!("{} ms", t.udp_idle_ms)), false))
                        .child(ui.info_row("Max UDP flows", div().child(t.max_udp_flows.to_string()), false)),
                )
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .mt(px(18.))
                        .pt(px(12.))
                        .border_t_1()
                        .border_color(p.line)
                        .child(
                            ui.button("duplicate-profile", "Duplicate", Some("copy"), ButtonKind::Ghost)
                                .on_click(cx.listener(move |this, _, _, cx| this.duplicate_profile(&ids.4, cx))),
                        )
                        .child(
                            ui.button("delete-profile", "Delete profile", None, ButtonKind::Danger)
                                .on_click(cx.listener({
                                    let name = profile.name.clone();
                                    move |this, _, window, cx| {
                                        let extra = if running_here { " The tunnel will disconnect first." } else { "" };
                                        this.open_confirm(
                                            &format!("Delete {name}?"),
                                            &format!("This removes the saved profile from this device.{extra} This action cannot be undone."),
                                            "Delete profile",
                                            "Keep profile",
                                            true,
                                            ConfirmAction::DeleteProfile(ids.5.clone()),
                                            window,
                                            cx,
                                        );
                                    }
                                })),
                        ),
                )
        });
        let wide = window.viewport_size().width > px(174. + 60. + 640.);
        let workspace = div()
            .flex()
            .gap(px(24.))
            .when(!wide, |this| this.flex_col())
            .child(list)
            .when_some(detail, |this, detail| this.child(detail));
        div().child(header).child(workspace)
    }

    pub fn test_profile(&mut self, id: String, cx: &mut Context<Self>) {
        let key = id.clone();
        self.call(
            Request::MeasureLatency { profile_id: id },
            cx,
            move |this, result, cx| match result {
                Ok(fiend_ipc::Reply::Latency { ms }) => {
                    this.model.latencies.insert(key, ms);
                }
                Ok(_) => {}
                Err(error) => this.show_toast(error, true, cx),
            },
        );
    }

    pub fn test_all(&mut self, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.model.profiles.iter().map(|p| p.id.clone()).collect();
        for id in ids {
            self.test_profile(id, cx);
        }
    }

    pub fn duplicate_profile(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(profile) = self.model.profile(id).cloned() else {
            return;
        };
        let input = fiend_ipc::ProfileInput {
            id: None,
            name: format!("{} copy", profile.name),
            servers: profile.servers,
            secret: profile.secret,
            template_json: profile.template_json,
            tuning: profile.tuning,
        };
        self.call(
            Request::SaveProfile { profile: input },
            cx,
            |this, result, cx| match result {
                Ok(fiend_ipc::Reply::Profile(profile)) => {
                    this.selected_profile = Some(profile.id);
                    this.show_toast("Profile duplicated", false, cx);
                }
                Ok(_) => {}
                Err(error) => this.show_toast(error, true, cx),
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn switch_row(
        &self,
        ui: &Ui,
        title: &str,
        description: &str,
        checked: bool,
        first: bool,
        change: impl Fn(&mut AppView, bool, &mut Context<AppView>) + 'static,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let view = cx.entity();
        let switch = ui
            .switch(
                crate::design::eid(format!("switch-{title}")),
                checked,
                title,
            )
            .on_change(move |checked, _, _, cx| {
                view.update(cx, |this, cx| change(this, checked, cx));
            });
        ui.setting_row(title, Some(description), switch, first)
    }

    fn set_daemon_setting(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut fiend_ipc::Settings),
    ) {
        let mut settings = self.model.settings.clone();
        change(&mut settings);
        self.call_toast(Request::SetSettings { settings }, cx);
    }

    pub fn page_routing(
        &mut self,
        ui: &Ui,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let p = ui.p;
        let actions = vec![
            ui.button(
                "add-subscription",
                "Add subscription",
                Some("plus"),
                ButtonKind::Default,
            )
            .on_click(cx.listener(|this, _, window, cx| this.open_subscription(window, cx)))
            .into_any_element(),
        ];
        let settings = self.model.settings.clone();
        let default_route = match settings.default_route {
            fiend_ipc::DefaultRoute::Tunnel => ("tunnel", "Tunnel (default)"),
            fiend_ipc::DefaultRoute::Direct => ("direct", "Direct"),
        };
        let default_route_select = self.select(
            ui,
            MenuId::DefaultRoute,
            default_route.1.to_owned(),
            vec![
                MenuOption::new("tunnel", "Tunnel (default)"),
                MenuOption::new("direct", "Direct"),
            ],
            default_route.0.to_owned(),
            false,
            160.,
            cx,
        );
        let cards = [
            (
                RuleKind::Proxy,
                "shield",
                "Wins over bypass rules",
                rule_count(&settings.proxy_rules),
            ),
            (
                RuleKind::Direct,
                "route",
                "Uses your network address",
                rule_count(&settings.direct_rules),
            ),
            (
                RuleKind::Dpi,
                "swap",
                "Direct, splits the site name",
                rule_count(&settings.dpi_rules),
            ),
            (
                RuleKind::Block,
                "block",
                "No DNS resolution",
                rule_count(&settings.block_rules),
            ),
        ]
        .into_iter()
        .map(|(kind, icon, note, count)| {
            Button::new(crate::design::eid(format!(
                "rule-card-{}",
                kind_value(kind)
            )))
            .accessibility_label(kind_label(kind))
            .relative()
            .flex_1()
            .min_w_0()
            .flex_col()
            .items_start()
            .justify_start()
            .p(px(17.))
            .rounded(px(5.))
            .bg(p.panel)
            .border_1()
            .border_color(p.line)
            .cursor_pointer()
            .hover(move |s| s.bg(p.soft).border_color(p.accent))
            .focus_visible(move |s| s.border_color(p.accent))
            .on_click(cx.listener(move |this, _, window, cx| this.open_rules(kind, window, cx)))
            .child(div().mb(px(12.)).child(ui.icon(
                icon,
                19.,
                if kind == RuleKind::Block {
                    p.danger
                } else {
                    p.muted
                },
            )))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(kind_label(kind)),
            )
            .child(
                ui.number_with_unit(&count.to_string(), 25., "rules", 10., 4.)
                    .mt(px(8.)),
            )
            .child(
                div()
                    .mt(px(3.))
                    .text_size(px(10.))
                    .text_color(p.muted)
                    .child(note),
            )
            .child(
                div()
                    .absolute()
                    .top(px(15.))
                    .right(px(15.))
                    .child(ui.icon("chevron", 13., p.muted)),
            )
        });
        let subscriptions: Vec<_> = settings.subscriptions.clone();
        let sub_rows = subscriptions.iter().enumerate().map(|(index, sub)| {
            let id = sub.id.clone();
            let edit_id = sub.id.clone();
            let remove_id = sub.id.clone();
            let title = sub.title();
            let when = if sub.updated_at_ms == 0 {
                "Not downloaded".to_owned()
            } else {
                relative_time(self.now_ms.saturating_sub(sub.updated_at_ms))
            };
            let detail = match &sub.error {
                Some(error) => format!("{} · {error}", kind_label(sub.kind)),
                None if sub.entries > 0 => {
                    format!("{} · {} rules · {when}", kind_label(sub.kind), sub.entries)
                }
                None => format!("{} · {when}", kind_label(sub.kind)),
            };
            div()
                .flex()
                .items_center()
                .gap(px(12.))
                .px(px(14.))
                .py(px(13.))
                .when(index > 0, |this| this.border_t_1().border_color(p.line))
                .child(ui.icon("link", 17., p.muted))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .text_size(px(11.))
                                .font_weight(FontWeight::MEDIUM)
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .child(title),
                        )
                        .child(
                            div()
                                .mt(px(4.))
                                .text_size(px(10.))
                                .text_color(if sub.error.is_some() {
                                    p.danger
                                } else {
                                    p.muted
                                })
                                .child(detail),
                        ),
                )
                .child(
                    ui.icon_button(
                        crate::design::eid(format!("edit-sub-{edit_id}")),
                        "edit",
                        "Edit subscription",
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_subscription_edit(&edit_id, window, cx)
                    })),
                )
                .child(
                    ui.icon_button(
                        crate::design::eid(format!("refresh-sub-{id}")),
                        "refresh",
                        "Refresh subscription",
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.refresh_subscriptions(cx))),
                )
                .child(
                    ui.icon_button(
                        crate::design::eid(format!("remove-sub-{remove_id}")),
                        "close",
                        "Remove subscription",
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_confirm(
                            "Remove subscription?",
                            "The downloaded rule list is deleted from this device.",
                            "Remove",
                            "Keep",
                            true,
                            ConfirmAction::RemoveSubscription(remove_id.clone()),
                            window,
                            cx,
                        );
                    })),
                )
        });
        let resolver = if settings.direct_resolver.is_empty() {
            "System default".to_owned()
        } else {
            settings.direct_resolver.clone()
        };
        let mtu = settings.mtu.to_string();
        let restart = self.model.status.restart_required;
        div()
            .child(self.page_header(ui, "Routing", actions))
            .child(
                ui.section_heading(
                    "Local rules",
                    Some(
                        div()
                            .text_size(px(10.))
                            .text_color(if restart { p.accent } else { p.muted })
                            .child(if restart {
                                "Reconnect to apply changes"
                            } else {
                                "Reconnect after changes"
                            })
                            .into_any_element(),
                    ),
                ),
            )
            .child(div().flex().gap(px(12.)).mb(px(25.)).children(cards))
            .child(
                ui.section_heading(
                    "Subscriptions",
                    Some(
                        ui.compact_button(
                            "refresh-all",
                            "Refresh all",
                            Some("refresh"),
                            ButtonKind::Ghost,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_subscriptions(cx)))
                        .into_any_element(),
                    ),
                ),
            )
            .child(if subscriptions.is_empty() {
                ui.group().child(
                    div()
                        .py(px(30.))
                        .px(px(10.))
                        .text_size(px(11.))
                        .text_color(p.muted)
                        .flex()
                        .justify_center()
                        .child("No subscriptions. Add an HTTP or HTTPS rule list."),
                )
            } else {
                ui.group().children(sub_rows)
            })
            .child(ui.group_title("Network"))
            .child(
                ui.group()
                    .child(ui.setting_row(
                        "Traffic with no matching rule",
                        Some("Direct uses your network address without the tunnel."),
                        default_route_select,
                        true,
                    ))
                    .child(
                        ui.setting_row(
                            "Direct DNS resolver",
                            Some("Used only for sites that bypass the tunnel."),
                            ui.button("edit-resolver", &resolver, None, ButtonKind::Ghost)
                                .font_family(MONO)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_value(ValueKind::Resolver, window, cx)
                                })),
                            false,
                        ),
                    )
                    .child(
                        ui.setting_row(
                            "Tunnel MTU",
                            Some("Maximum packet size."),
                            ui.button("edit-mtu", &mtu, None, ButtonKind::Ghost)
                                .font_family(MONO)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.open_value(ValueKind::Mtu, window, cx)
                                })),
                            false,
                        ),
                    ),
            )
    }

    pub fn refresh_subscriptions(&mut self, cx: &mut Context<Self>) {
        self.call(
            Request::RefreshSubscriptions,
            cx,
            |this, result, cx| match result {
                Ok(_) => this.show_toast("Rule lists refreshed", false, cx),
                Err(error) => this.show_toast(error, true, cx),
            },
        );
    }

    pub fn page_logs(&mut self, ui: &Ui, _: &mut Window, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = ui.p;
        let paused = self.logs_paused;
        let source = self
            .frozen_logs
            .clone()
            .unwrap_or_else(|| self.model.logs.clone());
        let filter = self.log_filter;
        let entries: Vec<_> = source
            .into_iter()
            .filter(|entry| filter.matches(entry.level))
            .collect();
        let text: String = entries
            .iter()
            .map(|entry| {
                format!(
                    "{} {:?} {}",
                    format::clock(entry.timestamp_ms),
                    entry.level,
                    entry.message
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let actions = vec![
            ui.button("copy-logs", "Copy", Some("copy"), ButtonKind::Default)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                    this.show_toast("Log copied", false, cx);
                }))
                .into_any_element(),
            ui.button("clear-logs", "Clear", None, ButtonKind::Ghost)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.frozen_logs = this.frozen_logs.as_ref().map(|_| Vec::new());
                    this.call_toast(Request::ClearLogs, cx);
                    this.model.logs.clear();
                    cx.notify();
                }))
                .into_any_element(),
        ];
        let options: Vec<MenuOption> = [
            LogFilter::All,
            LogFilter::Info,
            LogFilter::Debug,
            LogFilter::Warn,
        ]
        .into_iter()
        .map(|f| MenuOption::new(f.value(), f.label()))
        .collect();
        let select = self.select(
            ui,
            MenuId::LogLevel,
            filter.label().to_owned(),
            options,
            filter.value().to_owned(),
            false,
            128.,
            cx,
        );
        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .mb(px(16.))
            .child(select)
            .child(
                ui.button(
                    "pause-logs",
                    if paused { "Resume" } else { "Pause" },
                    None,
                    ButtonKind::Default,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.logs_paused = !this.logs_paused;
                    this.frozen_logs = this.logs_paused.then(|| this.model.logs.clone());
                    cx.notify();
                })),
            )
            .child(
                div()
                    .ml_auto()
                    .text_size(px(10.))
                    .text_color(p.muted)
                    .child(format!("{} events", entries.len())),
            );
        let rows = entries
            .iter()
            .rev()
            .take(500)
            .enumerate()
            .map(|(index, entry)| {
                let (label, color) = match entry.level {
                    LogLevel::Warn | LogLevel::Error => ("WARN", p.danger),
                    LogLevel::Info => ("INFO", p.accent),
                    _ => ("DEBUG", p.accent),
                };
                div()
                    .id(("log", index))
                    .flex()
                    .items_start()
                    .py(px(11.))
                    .border_b_1()
                    .border_color(p.line)
                    .font_family(MONO)
                    .text_size(px(10.))
                    .hover(move |s| s.bg(p.soft))
                    .child(
                        div()
                            .w(px(100.))
                            .px(px(10.))
                            .flex_none()
                            .text_size(px(9.))
                            .text_color(p.muted)
                            .child(format::clock(entry.timestamp_ms)),
                    )
                    .child(
                        div()
                            .w(px(63.))
                            .flex_none()
                            .text_size(px(9.))
                            .text_color(color)
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pr(px(10.))
                            .child(entry.message.clone()),
                    )
            });
        div()
            .child(self.page_header(ui, "Logs", actions))
            .child(toolbar)
            .child(
                div()
                    .flex()
                    .py(px(9.))
                    .border_b_1()
                    .border_color(p.line)
                    .text_size(px(10.))
                    .text_color(p.muted)
                    .font_weight(FontWeight::MEDIUM)
                    .child(div().w(px(100.)).px(px(10.)).flex_none().child("Time"))
                    .child(div().w(px(63.)).flex_none().child("Level"))
                    .child(div().flex_1().child("Event")),
            )
            .child(if entries.is_empty() {
                div()
                    .py(px(30.))
                    .flex()
                    .justify_center()
                    .text_size(px(11.))
                    .text_color(p.muted)
                    .child("No events.")
            } else {
                div().children(rows)
            })
    }

    pub fn page_settings(
        &mut self,
        ui: &Ui,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::Div {
        let p = ui.p;
        let tabs = [
            (SettingsTab::General, "General"),
            (SettingsTab::Appearance, "Appearance"),
            (SettingsTab::About, "About"),
        ]
        .into_iter()
        .map(|(tab, label)| {
            let selected = self.settings_tab == tab;
            Button::new(crate::design::eid(format!("{}-{}", "settings-tab", label)))
                .accessibility_label(label)
                .px(px(14.))
                .pt(px(9.))
                .pb(px(11.))
                .text_size(px(11.))
                .cursor_pointer()
                .border_b_2()
                .border_color(if selected {
                    gpui_kit::Hsla::from(p.accent)
                } else {
                    gpui_kit::transparent_black()
                })
                .text_color(if selected { p.accent } else { p.muted })
                .when(selected, |this| this.font_weight(FontWeight::MEDIUM))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.settings_tab = tab;
                    cx.notify();
                }))
                .child(label)
        })
        .collect::<Vec<_>>();
        let body = match self.settings_tab {
            SettingsTab::General => self.settings_general(ui, cx).into_any_element(),
            SettingsTab::Appearance => self.settings_appearance(ui, cx).into_any_element(),
            SettingsTab::About => self.settings_about(ui).into_any_element(),
        };
        let _ = window;
        div()
            .max_w(px(660.))
            .child(self.page_header(ui, "Settings", Vec::new()))
            .child(
                div()
                    .flex()
                    .gap(px(4.))
                    .mb(px(24.))
                    .border_b_1()
                    .border_color(p.line)
                    .children(tabs),
            )
            .child(body)
    }

    fn settings_general(&mut self, ui: &Ui, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = ui.p;
        let settings = self.model.settings.clone();
        let prefs = self.prefs.clone();
        let service_ready = self.model.service.is_ready();
        let installed = crate::service::is_installed();
        let outdated = service_ready && self.service_outdated;
        let status = if outdated {
            "Installed · Update available".to_owned()
        } else if service_ready {
            "Installed · Ready to create a tunnel".to_owned()
        } else {
            match self.model.service {
                ServiceState::NotInstalled => {
                    "Not installed · Administrator access is required".into()
                }
                ServiceState::NotRunning => "Installed · Not running".into(),
                ServiceState::Refused(ref message) => message.clone(),
                ServiceState::Incompatible(ref message) => message.clone(),
                _ => "Checking…".into(),
            }
        };
        let service_control = if service_ready {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .when(!outdated, |this| this.child(ui.tag("Ready", true)))
                .when(outdated, |this| {
                    this.child(
                        ui.compact_button("update-service", "Update", None, ButtonKind::Primary)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_service_dialog(window, cx)
                            })),
                    )
                })
                .when(installed, |this| {
                    this.child(ui.compact_button("uninstall-service", "Uninstall", None, ButtonKind::Ghost).on_click(
                        cx.listener(|this, _, window, cx| {
                            this.open_confirm(
                                "Uninstall the service?",
                                "The tunnel stops and the original routes and DNS return. Profiles and settings stay on this device.",
                                "Uninstall",
                                "Keep service",
                                true,
                                ConfirmAction::UninstallService,
                                window,
                                cx,
                            );
                        }),
                    ))
                })
        } else if matches!(
            self.model.service,
            ServiceState::NotInstalled | ServiceState::NotRunning
        ) {
            let label = if matches!(self.model.service, ServiceState::NotRunning) {
                "Start"
            } else {
                "Install"
            };
            div().child(
                ui.button("install-service", label, None, ButtonKind::Primary)
                    .on_click(
                        cx.listener(|this, _, window, cx| this.open_service_dialog(window, cx)),
                    ),
            )
        } else {
            div()
        };
        div()
            .child(ui.group_title("Startup"))
            .child(
                ui.group()
                    .child(self.switch_row(
                        ui,
                        "Launch at login",
                        "Open Fiend when you sign in.",
                        prefs.launch_at_login,
                        true,
                        |this, checked, cx| {
                            match crate::system::set_launch_at_login(checked) {
                                Ok(()) => {
                                    this.prefs.launch_at_login = checked;
                                    this.prefs.save();
                                }
                                Err(error) => this.show_toast(error, true, cx),
                            }
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(self.switch_row(
                        ui,
                        "Connect when Fiend opens",
                        "Use the last selected profile.",
                        prefs.connect_on_open,
                        false,
                        |this, checked, cx| {
                            this.prefs.connect_on_open = checked;
                            this.prefs.save();
                            cx.notify();
                        },
                        cx,
                    ))
                    .child(self.switch_row(
                        ui,
                        "Connect at system startup",
                        "The service can connect before you sign in.",
                        settings.connect_at_startup,
                        false,
                        |this, checked, cx| this.set_daemon_setting(cx, |s| s.connect_at_startup = checked),
                        cx,
                    )),
            )
            .child(ui.group_title("Connection"))
            .child(
                ui.group()
                    .child(self.switch_row(
                        ui,
                        "Bypass local network",
                        "Private ranges outside the local link bypass the tunnel.",
                        settings.bypass_lan,
                        true,
                        |this, checked, cx| this.set_daemon_setting(cx, |s| s.bypass_lan = checked),
                        cx,
                    ))
                    .child(self.switch_row(
                        ui,
                        "IPv6",
                        "Send IPv6 traffic through the tunnel.",
                        settings.ipv6,
                        false,
                        |this, checked, cx| this.set_daemon_setting(cx, |s| s.ipv6 = checked),
                        cx,
                    )),
            )
            .child(
                div()
                    .mx(px(14.))
                    .mt(px(10.))
                    .mb(px(20.))
                    .text_size(px(10.))
                    .text_color(p.muted)
                    .child("Your directly connected local network stays reachable. Reconnect to apply network changes."),
            )
            .child(ui.group_title("Backup"))
            .child(
                ui.group().child(ui.setting_row(
                    "Backup and restore",
                    Some("Move profiles, settings, and rule lists to another computer. The file is encrypted with a password."),
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            ui.compact_button("backup-start", "Back up", Some("data"), ButtonKind::Default)
                                .disabled(!service_ready)
                                .on_click(cx.listener(|this, _, window, cx| this.open_backup(window, cx))),
                        )
                        .child(
                            ui.compact_button("restore-start", "Restore", Some("down"), ButtonKind::Ghost)
                                .disabled(!service_ready)
                                .on_click(cx.listener(|this, _, window, cx| this.open_restore(window, cx))),
                        ),
                    true,
                )),
            )
            .child(ui.group_title("Service"))
            .child(ui.group().child(ui.setting_row("Fiend system service", Some(&status), service_control, true)))
    }

    fn settings_appearance(&mut self, ui: &Ui, cx: &mut Context<Self>) -> gpui_kit::Div {
        let p = ui.p;
        let current = self.prefs.theme;
        let choices = [
            (ThemeMode::Light, "Light"),
            (ThemeMode::Dark, "Dark"),
            (ThemeMode::System, "System"),
        ]
        .into_iter()
        .map(|(mode, label)| {
            let selected = current == mode;
            let sample = match mode {
                ThemeMode::Light => div()
                    .bg(gpui_kit::rgb(0xfafbfb))
                    .border_color(gpui_kit::rgb(0xd9dfe3))
                    .child(div().w(px(36.)).h_full().bg(gpui_kit::rgb(0xeff1f2)))
                    .child(
                        div()
                            .flex_1()
                            .m(px(12.))
                            .rounded(px(3.))
                            .bg(gpui_kit::rgb(0xe9edf0)),
                    ),
                ThemeMode::Dark => div()
                    .bg(gpui_kit::rgb(0x1b1e21))
                    .border_color(gpui_kit::rgb(0x353d43))
                    .child(div().w(px(36.)).h_full().bg(gpui_kit::rgb(0x171a1d)))
                    .child(
                        div()
                            .flex_1()
                            .m(px(12.))
                            .rounded(px(3.))
                            .bg(gpui_kit::rgb(0x2a3035)),
                    ),
                ThemeMode::System => div()
                    .bg(gpui_kit::rgb(0xfafbfb))
                    .border_color(gpui_kit::rgb(0xd9dfe3))
                    .child(div().w(px(36.)).h_full().bg(gpui_kit::rgb(0xeff1f2)))
                    .child(
                        div()
                            .flex_1()
                            .m(px(12.))
                            .rounded(px(3.))
                            .bg(gpui_kit::rgb(0x2a3035)),
                    ),
            };
            Button::new(crate::design::eid(format!("{}-{}", "theme", label)))
                .accessibility_label(label)
                .flex_1()
                .flex_col()
                .items_stretch()
                .p(px(9.))
                .rounded(px(5.))
                .border_1()
                .border_color(if selected { p.accent } else { p.line })
                .text_size(px(11.))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, _, cx| this.set_theme(mode, cx)))
                .child(
                    sample
                        .flex()
                        .h(px(72.))
                        .mb(px(8.))
                        .rounded(px(3.))
                        .border_1()
                        .overflow_hidden(),
                )
                .child(
                    div()
                        .flex()
                        .justify_between()
                        .items_center()
                        .child(label)
                        .when(selected, |this| this.child(ui.icon("check", 13., p.accent))),
                )
        });
        div().child(ui.group_title("Theme")).child(
            div()
                .flex()
                .gap(px(12.))
                .mt(px(16.))
                .mb(px(20.))
                .children(choices),
        )
    }

    fn settings_about(&mut self, ui: &Ui) -> gpui_kit::Div {
        let p = ui.p;
        div()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(14.))
                    .mt(px(28.))
                    .mb(px(20.))
                    .child(
                        div()
                            .relative()
                            .size(px(28.))
                            .child(ui.icon("brand", 28., p.text).absolute().top_0().left_0())
                            .child(
                                ui.icon("brand-face", 28., p.surface)
                                    .absolute()
                                    .top_0()
                                    .left_0(),
                            ),
                    )
                    .child(
                        div()
                            .child(
                                div()
                                    .text_size(px(23.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child("fiend"),
                            )
                            .child(div().text_size(px(11.)).text_color(p.muted).child(format!(
                                "Version {} · Service {}",
                                env!("CARGO_PKG_VERSION"),
                                if self.model.status.daemon_version.is_empty() {
                                    "not connected".to_owned()
                                } else {
                                    self.model.status.daemon_version.clone()
                                }
                            ))),
                    ),
            )
            .child(
                ui.group()
                    .child(
                        ui.setting_row(
                            "GitHub",
                            Some("rfc9849/Fiend"),
                            ui.compact_button("about-github", "Open", None, ButtonKind::Default)
                                .accessibility_label("Open Fiend on GitHub")
                                .on_click(|_, _, cx| {
                                    cx.open_url("https://github.com/rfc9849/Fiend")
                                }),
                            true,
                        ),
                    )
                    .child(
                        ui.setting_row(
                            "License",
                            Some("PolyForm Noncommercial 1.0.0"),
                            ui.compact_button("about-license", "Open", None, ButtonKind::Default)
                                .accessibility_label("Open the Fiend license")
                                .on_click(|_, _, cx| {
                                    cx.open_url(
                                        "https://github.com/rfc9849/Fiend/blob/master/LICENSE",
                                    )
                                }),
                            false,
                        ),
                    ),
            )
    }
}

fn relative_time(elapsed_ms: u64) -> String {
    let seconds = elapsed_ms / 1000;
    let (count, unit) = match seconds {
        0..=59 => return "just now".to_owned(),
        60..=3599 => (seconds / 60, "minute"),
        3600..=86399 => (seconds / 3600, "hour"),
        _ => (seconds / 86400, "day"),
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_are_readable() {
        assert_eq!(relative_time(10_000), "just now");
        assert_eq!(relative_time(12 * 60_000), "12 minutes ago");
        assert_eq!(relative_time(5 * 3_600_000), "5 hours ago");
        assert_eq!(relative_time(3 * 86_400_000), "3 days ago");
        assert_eq!(relative_time(60_000), "1 minute ago");
        assert_eq!(relative_time(3_600_000), "1 hour ago");
        assert_eq!(relative_time(26 * 3_600_000), "1 day ago");
    }
}
