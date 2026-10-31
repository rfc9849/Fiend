use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use fiend_daemon::peer::{PeerPolicy, SystemDirectory};
use fiend_daemon::platform::NullPlatform;
use fiend_daemon::{Daemon, LogBuffer, Store, bind_socket, serve};
use fiend_desktop::{AppView, Assets, Dialog, Page, Prefs, ServiceState, spawn_backend};
use fiend_ipc::{RuleKind, TunnelState};
use gpui_kit::{
    AppContext as _, Context, Entity, HeadlessAppContext, Window, WindowHandle, px, size,
};
use tokio_util::sync::CancellationToken;

const TEMPLATE: &str = include_str!("../../../fiend-core/testdata/template.json");

struct Backend {
    socket: PathBuf,
    server: std::net::SocketAddr,
    stop_daemon: CancellationToken,
    dir: PathBuf,
}

fn start_backend() -> Backend {
    let dir = std::env::temp_dir().join(format!("fiend-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let socket = dir.join("d.sock");
    let stop_daemon = CancellationToken::new();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let thread_dir = dir.clone();
    let thread_socket = socket.clone();
    let stop = stop_daemon.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let server_addr = listener.local_addr().unwrap();
            let server = managed_server(
                &[0x42; 32],
                fiend_core::server::ServerConfig::new("127.0.0.1", 443),
            );
            tokio::spawn(async move {
                let _ = server.serve(listener).await;
            });
            let store = Store::open(thread_dir.join("store")).unwrap();
            let daemon = Daemon::new(NullPlatform, store, Arc::new(LogBuffer::new()));
            let unix = bind_socket(&thread_socket, None, &SystemDirectory).unwrap();
            let policy = PeerPolicy {
                console: true,
                uids: vec![current_uid()],
                group: None,
            };
            let shutdown = CancellationToken::new();
            tokio::spawn(serve(daemon.clone(), unix, policy, shutdown.clone()));
            ready_tx.send(server_addr).unwrap();
            stop.cancelled().await;
            daemon.shutdown().await;
            shutdown.cancel();
            let _ = std::fs::remove_file(&thread_socket);
            tokio::time::sleep(Duration::from_secs(30)).await;
        });
    });
    let server = ready_rx.recv().unwrap();
    Backend {
        socket,
        server,
        stop_daemon,
        dir,
    }
}

fn managed_server(
    secret: &[u8],
    config: fiend_core::server::ServerConfig,
) -> fiend_core::server::Server {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "fiend-desktop-test-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let mut state = fiend_core::node::DesiredState::initial(&config);
    state.users.push(fiend_core::node::User {
        id: "test".into(),
        secret: secret.iter().map(|byte| format!("{byte:02x}")).collect(),
        enabled: true,
        allowed: true,
        expire_at: None,
        traffic_limit: None,
        used_traffic: 0,
        usage_epoch: 0,
    });
    let node = fiend_core::node::Node::load(&path, vec![0x5c; 32], state).unwrap();
    fiend_core::server::Server::managed(config, node)
}

fn current_uid() -> u32 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(std::env::temp_dir()).unwrap().uid()
}

struct Harness {
    cx: HeadlessAppContext,
    window: WindowHandle<AppView>,
    view: Entity<AppView>,
}

impl Harness {
    fn with<R>(
        &mut self,
        f: impl FnOnce(&mut AppView, &mut Window, &mut Context<AppView>) -> R,
    ) -> R {
        let view = self.view.clone();
        let mut result = None;
        self.cx
            .update_window(self.window.into(), |_, window, cx| {
                view.update(cx, |view, cx| result = Some(f(view, window, cx)));
            })
            .unwrap();
        self.cx.run_until_parked();
        result.unwrap()
    }

    fn wait(&mut self, what: &str, mut check: impl FnMut(&mut AppView) -> bool) {
        let start = Instant::now();
        loop {
            self.cx.run_until_parked();
            if self.with(|view, _, _| check(view)) {
                return;
            }
            assert!(start.elapsed() < Duration::from_secs(20), "timeout: {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn set_input(
        &mut self,
        pick: impl FnOnce(&mut AppView) -> Entity<gpui_kit::base::input::InputState>,
        text: &str,
    ) {
        let text = text.to_owned();
        self.with(|view, window, cx| {
            let state = pick(view);
            state.update(cx, |state, cx| state.set_value(text, window, cx));
        });
    }
}

fn profile_form(view: &mut AppView) -> &mut fiend_desktop::ProfileForm {
    match view.dialog.as_mut() {
        Some(Dialog::Profile(form)) => form,
        _ => panic!("the profile form is not open"),
    }
}

pub fn main() {
    let backend = start_backend();
    let platform = gpui_kit::platform::current_platform(true);
    let mut cx = HeadlessAppContext::with_platform(
        platform.text_system(),
        Arc::new(Assets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(gpui_kit::init);
    let (client, updates) = spawn_backend(backend.socket.clone());
    let window = cx
        .open_window(size(px(1000.), px(690.)), |window, cx| {
            cx.new(|cx| AppView::new(window, cx, client, updates, Prefs::default()))
        })
        .unwrap();
    let view = window.root(&mut cx).unwrap();
    let mut h = Harness { cx, window, view };

    h.wait("service ready", |v| v.model.service.is_ready());
    println!("ok: service is ready");

    h.with(|v, w, cx| v.toggle_connection(w, cx));
    assert!(
        h.with(|v, _, _| v.dialog.is_some()),
        "no profiles: the form opens"
    );
    println!("ok: an empty profile list opens the form");

    h.with(|v, w, cx| {
        v.close_dialog(w, cx);
        v.open_profile_form(None, w, cx);
    });
    h.set_input(|v| profile_form(v).name.clone(), "Local");
    h.set_input(|v| profile_form(v).host.clone(), "127.0.0.1");
    h.set_input(
        |v| profile_form(v).port.clone(),
        &backend.server.port().to_string(),
    );
    h.set_input(|v| profile_form(v).secret.clone(), "short");
    h.with(|v, w, cx| v.submit_dialog(w, cx));
    assert!(h.with(|v, _, _| profile_form(v).errors.contains_key("secret")));
    println!("ok: a bad secret is refused below the field");

    h.set_input(|v| profile_form(v).secret.clone(), &"42".repeat(32));
    h.with(|v, w, cx| {
        profile_form(v).template_json = TEMPLATE.to_owned();
        v.submit_dialog(w, cx);
    });
    h.wait("profile saved", |v| {
        v.dialog.is_none() && v.model.profiles.len() == 1
    });
    println!("ok: the profile is saved");

    h.with(|v, w, cx| v.toggle_connection(w, cx));
    h.wait("connected", |v| {
        matches!(v.model.status.state, TunnelState::Connected { .. })
    });
    assert!(h.with(|v, _, _| v.model.status.profile_id.is_some()));
    println!("ok: the power action connects");

    h.with(|v, w, cx| v.toggle_connection(w, cx));
    h.wait("disconnected", |v| {
        matches!(v.model.status.state, TunnelState::Disconnected)
    });
    println!("ok: the power action disconnects");

    h.with(|v, w, cx| v.open_rules(RuleKind::Block, w, cx));
    h.with(|v, w, cx| {
        let Some(Dialog::Rules(form)) = v.dialog.as_ref() else {
            panic!("no rules form")
        };
        let state = form.text.clone();
        state.update(cx, |state, cx| {
            state.set_value("regexp:x".to_owned(), w, cx)
        });
        v.on_dialog_input_change(cx);
    });
    h.cx.run_until_parked();
    assert!(
        h.with(|v, _, _| matches!(v.dialog.as_ref(), Some(Dialog::Rules(f)) if f.error.is_some()))
    );
    println!("ok: invalid rules show an error before saving");
    h.with(|v, w, cx| {
        let Some(Dialog::Rules(form)) = v.dialog.as_ref() else {
            panic!()
        };
        let state = form.text.clone();
        state.update(cx, |state, cx| {
            state.set_value("domain:ads.example.com\n".to_owned(), w, cx)
        });
        v.on_dialog_input_change(cx);
    });
    h.cx.run_until_parked();
    h.with(|v, w, cx| v.submit_dialog(w, cx));
    h.wait("rules saved", |v| {
        v.dialog.is_none() && v.model.settings.block_rules.contains("ads.example.com")
    });
    println!("ok: valid rules are saved by the daemon");

    h.with(|v, w, cx| {
        v.navigate(Page::Settings, cx);
        let _ = w;
    });
    backend.stop_daemon.cancel();
    h.wait("service lost", |v| !v.model.service.is_ready());
    let state = h.with(|v, _, _| v.model.service.clone());
    assert!(
        matches!(state, ServiceState::NotInstalled | ServiceState::NotRunning),
        "{state:?}"
    );
    println!("ok: a stopped daemon shows the service state");

    let _ = std::fs::remove_dir_all(&backend.dir);
    println!("e2e ok");
}
