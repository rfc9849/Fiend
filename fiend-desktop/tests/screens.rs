use std::path::PathBuf;
use std::sync::Arc;

use fiend_desktop::{
    AppView, Assets, Backend, Page, Prefs, ServiceState, SettingsTab, ThemeMode, Update, ValueKind,
};
use fiend_ipc::{
    LogEntry, LogLevel, Profile, Settings, Stats, Status, Subscription, Tuning, TunnelState,
};
use futures::channel::mpsc;
use gpui_kit::prelude::*;
use gpui_kit::{HeadlessAppContext, IntoElement, Render, div, px, rgb, size, svg};

const TEMPLATE: &str = include_str!("../../fiend-core/testdata/template.json");

pub fn profiles() -> Vec<Profile> {
    ["Amsterdam", "Helsinki", "Frankfurt", "Stockholm"]
        .iter()
        .enumerate()
        .map(|(index, name)| Profile {
            id: format!("{index:032x}"),
            name: (*name).into(),
            server: format!("{}.fiend.example:443", name[..2].to_lowercase()),
            secret: format!("{:02x}", index + 10).repeat(32),
            template_json: TEMPLATE.into(),
            tuning: Tuning::default(),
        })
        .collect()
}

pub fn logs() -> Vec<LogEntry> {
    [
        (LogLevel::Info, "Service ready. Waiting for a connection."),
        (LogLevel::Info, "tunnel start profile=Amsterdam"),
        (LogLevel::Info, "server handshake completed latency_ms=24"),
        (LogLevel::Warn, "rule list refresh failed reason=HTTP 404"),
        (LogLevel::Info, "tunnel connected"),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (level, message))| LogEntry {
        timestamp_ms: 34_862_018 + index as u64 * 140,
        level,
        message: message.into(),
    })
    .collect()
}

pub fn connected_updates() -> Vec<Update> {
    let list = profiles();
    let active = list[0].id.clone();
    vec![
        Update::Service(ServiceState::Ready),
        Update::Profiles(list),
        Update::Settings(Settings {
            active_profile: Some(active.clone()),
            direct_rules: "domain:example.org\nip:192.168.0.0/16\nip:10.0.0.0/8".into(),
            block_rules: "domain:ads.example.com\nfull:tracker.example.org\nkeyword:adserver"
                .into(),
            proxy_rules:
                "domain:youtube.com\ndomain:github.com\ndomain:wikipedia.org\nfull:api.github.com"
                    .into(),
            subscriptions: vec![Subscription {
                id: "s1".into(),
                url: "https://rules.example/privacy.txt".into(),
                kind: fiend_ipc::RuleKind::Block,
                updated_at_ms: 1,
                entries: 1842,
                error: None,
            }],
            ..Settings::default()
        }),
        Update::Status(Status {
            state: TunnelState::Connected { since_ms: 1 },
            profile_id: Some(active),
            stats: Some(Stats {
                sent_bytes: 120_000_000,
                received_bytes: 1_120_000_000,
                upload_bps: 400_000,
                download_bps: 3_100_000,
                active_flows: 12,
                dropped_packets: 0,
                latency_ms: Some(24),
            }),
            restart_required: false,
            daemon_version: "0.1.0".into(),
        }),
        Update::Logs(logs()),
    ]
}

pub struct Shot {
    pub name: &'static str,
    pub width: f32,
    pub height: f32,
}

pub fn render(
    shot: &Shot,
    theme: ThemeMode,
    updates: Vec<Update>,
    drive: impl FnOnce(&mut AppView, &mut gpui_kit::Window, &mut gpui_kit::Context<AppView>),
) -> PathBuf {
    let platform = gpui_kit::platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(
        platform.text_system(),
        Arc::new(Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(gpui_kit::init);
    let (sender, receiver) = mpsc::unbounded();
    let backend = Backend::offline();
    let prefs = Prefs {
        theme,
        ..Prefs::default()
    };
    let window = cx
        .open_window(size(px(shot.width), px(shot.height)), |window, cx| {
            cx.new(|cx| AppView::new(window, cx, backend, receiver, prefs))
        })
        .unwrap();
    for update in updates {
        sender.unbounded_send(update).unwrap();
    }
    cx.run_until_parked();
    let view = window.root(&mut cx).unwrap();
    cx.update_window(window.into(), |_, window, cx| {
        view.update(cx, |view, cx| drive(view, window, cx));
    })
    .unwrap();
    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into()).unwrap();
    let dir = std::env::temp_dir().join("fiend-shots");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{}.png", shot.name));
    image.save(&path).unwrap();
    path
}

fn shot(
    name: &'static str,
    theme: ThemeMode,
    updates: Vec<Update>,
    drive: impl FnOnce(&mut AppView, &mut gpui_kit::Window, &mut gpui_kit::Context<AppView>),
) {
    let path = render(
        &Shot {
            name,
            width: 1000.,
            height: 690.,
        },
        theme,
        updates,
        drive,
    );
    assert!(path.exists());
}

fn disconnected() -> Vec<Update> {
    let mut updates = connected_updates();
    updates.push(Update::Status(Status {
        state: TunnelState::Disconnected,
        profile_id: None,
        stats: None,
        restart_required: false,
        daemon_version: "0.1.0".into(),
    }));
    updates
}

struct Icon;

impl Render for Icon {
    fn render(
        &mut self,
        _: &mut gpui_kit::Window,
        _: &mut gpui_kit::Context<Self>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .size(px(480.))
                    .rounded(px(108.))
                    .bg(rgb(0x171a1d))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .relative()
                            .size(px(300.))
                            .child(
                                svg()
                                    .path("icons/brand.svg")
                                    .size(px(300.))
                                    .text_color(rgb(0xe2e7eb))
                                    .absolute()
                                    .top_0()
                                    .left_0(),
                            )
                            .child(
                                svg()
                                    .path("icons/brand-face.svg")
                                    .size(px(300.))
                                    .text_color(rgb(0x171a1d))
                                    .absolute()
                                    .top_0()
                                    .left_0(),
                            ),
                    ),
            )
    }
}

fn write_icon() {
    let platform = gpui_kit::platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(
        platform.text_system(),
        Arc::new(Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(gpui_kit::init);
    let window = cx
        .open_window(size(px(512.), px(512.)), |_, cx| cx.new(|_| Icon))
        .unwrap();
    cx.run_until_parked();
    let image = cx.capture_screenshot(window.into()).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/icon-1024.png");
    image.save(&path).unwrap();
    println!("icon written to {}", path.display());
}

fn main() {
    if std::env::var_os("FIEND_WRITE_ICON").is_some() {
        write_icon();
        return;
    }
    for (suffix, theme) in [("light", ThemeMode::Light), ("dark", ThemeMode::Dark)] {
        let n =
            |base: &str| -> &'static str { Box::leak(format!("{base}-{suffix}").into_boxed_str()) };
        shot(n("home"), theme, connected_updates(), |_, _, _| {});
        shot(n("home-off"), theme, disconnected(), |_, _, _| {});
        shot(n("profiles"), theme, connected_updates(), |v, _, cx| {
            v.navigate(Page::Profiles, cx)
        });
        shot(n("routing"), theme, connected_updates(), |v, _, cx| {
            v.navigate(Page::Routing, cx)
        });
        shot(n("logs"), theme, connected_updates(), |v, _, cx| {
            v.navigate(Page::Logs, cx)
        });
        shot(n("settings"), theme, connected_updates(), |v, _, cx| {
            v.navigate(Page::Settings, cx)
        });
        shot(n("appearance"), theme, connected_updates(), |v, _, cx| {
            v.navigate(Page::Settings, cx);
            v.settings_tab = SettingsTab::Appearance;
        });
        shot(n("about"), theme, connected_updates(), |v, _, cx| {
            v.navigate(Page::Settings, cx);
            v.settings_tab = SettingsTab::About;
        });
        shot(n("profile-form"), theme, connected_updates(), |v, w, cx| {
            v.open_profile_form(None, w, cx)
        });
        shot(n("profile-edit"), theme, connected_updates(), |v, w, cx| {
            let id = v.model.profiles[0].id.clone();
            v.open_profile_form(Some(id), w, cx)
        });
        shot(n("profile-imported"), theme, connected_updates(), |v, w, cx| {
            let template = fiend_core::fingerprint::Template::from_json(TEMPLATE).unwrap();
            v.model.profiles[1].template_json = serde_json::to_string(&template).unwrap();
            let id = v.model.profiles[1].id.clone();
            v.open_profile_form(Some(id), w, cx)
        });
        shot(n("import"), theme, connected_updates(), |v, w, cx| {
            v.open_import(None, w, cx)
        });
        shot(n("rules"), theme, connected_updates(), |v, w, cx| {
            v.open_rules(fiend_ipc::RuleKind::Proxy, w, cx)
        });
        shot(n("subscription"), theme, connected_updates(), |v, w, cx| {
            v.open_subscription(w, cx)
        });
        shot(n("mtu"), theme, connected_updates(), |v, w, cx| {
            v.open_value(ValueKind::Mtu, w, cx)
        });
        shot(
            n("service"),
            theme,
            vec![Update::Service(ServiceState::NotInstalled)],
            |v, w, cx| v.open_service_dialog(w, cx),
        );
        shot(
            n("no-service"),
            theme,
            vec![Update::Service(ServiceState::NotInstalled)],
            |_, _, _| {},
        );
        shot(
            n("empty"),
            theme,
            vec![Update::Service(ServiceState::Ready)],
            |v, _, cx| v.navigate(Page::Profiles, cx),
        );
        shot(
            n("failed"),
            theme,
            {
                let mut u = connected_updates();
                u.push(Update::Status(Status {
                    state: TunnelState::Failed {
                        reason: "Server did not respond.".into(),
                    },
                    profile_id: None,
                    stats: None,
                    restart_required: false,
                    daemon_version: "0.1.0".into(),
                }));
                u
            },
            |_, _, _| {},
        );
        shot(
            n("connecting"),
            theme,
            {
                let mut u = connected_updates();
                u.push(Update::Status(Status {
                    state: TunnelState::Connecting {
                        step: fiend_ipc::ConnectStep::Routes,
                    },
                    profile_id: None,
                    stats: None,
                    restart_required: false,
                    daemon_version: "0.1.0".into(),
                }));
                u
            },
            |_, _, _| {},
        );
        shot(n("menu"), theme, connected_updates(), |v, w, cx| {
            let options = v
                .model
                .profiles
                .iter()
                .map(|p| {
                    fiend_desktop::MenuOption::new(p.id.clone(), p.name.clone())
                        .detail(p.server.clone())
                })
                .collect();
            let selected = v.model.profiles[0].id.clone();
            v.open_menu(fiend_desktop::MenuId::Profile, options, &selected, w, cx);
        });
    }
    println!("screens ok");
}
