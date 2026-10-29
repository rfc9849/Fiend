use std::collections::HashMap;

use fiend_ipc::{ProfileInput, Reply, Request, RuleKind, Settings, Tuning};
use gpui_kit::base::input::{InputEvent, InputState, TextareaState};
use gpui_kit::base::{FocusTrapElement as _, Input, Textarea};
use gpui_kit::prelude::*;
use gpui_kit::{
    ClipboardItem, Context, Entity, ExternalPaths, Focusable as _, FontWeight, IntoElement, Window,
    canvas, div, fill, point, px, size,
};

use crate::app::{AppView, Page};
use crate::design::{ButtonKind, MONO, Ui};
use crate::format;
use crate::menu::{MenuId, MenuOption};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ValueKind {
    Resolver,
    Mtu,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ConfirmAction {
    DeleteProfile(String),
    RemoveSubscription(String),
    UninstallService,
    DiscardProfile,
    DiscardRules,
}

pub struct ProfileForm {
    pub id: Option<String>,
    pub name: Entity<InputState>,
    pub host: Entity<InputState>,
    pub port: Entity<InputState>,
    pub secret: Entity<InputState>,
    pub tuning: [Entity<InputState>; 5],
    pub template: String,
    pub template_json: String,
    pub errors: HashMap<&'static str, String>,
    pub show_secret: bool,
    pub tuning_open: bool,
    pub baseline: String,
    pub saving: bool,
    pub form_error: Option<String>,
}

pub struct ImportForm {
    pub link: Entity<TextareaState>,
    pub error: Option<String>,
    pub busy: bool,
}

pub struct ShareView {
    pub name: String,
    pub link: String,
    pub qr: Option<(usize, Vec<bool>)>,
    pub note: Option<String>,
}

pub struct RulesForm {
    pub kind: RuleKind,
    pub text: Entity<TextareaState>,
    pub error: Option<String>,
    pub count: usize,
    pub baseline: String,
    pub busy: bool,
}

pub struct SubscriptionForm {
    pub url: Entity<InputState>,
    pub kind: String,
    pub error: Option<String>,
    pub busy: bool,
}

pub struct ValueForm {
    pub kind: ValueKind,
    pub input: Entity<InputState>,
    pub error: Option<String>,
}

pub struct ConfirmDialog {
    pub title: String,
    pub body: String,
    pub confirm: String,
    pub cancel: String,
    pub danger: bool,
    pub action: ConfirmAction,
    pub busy: bool,
}

pub struct ServiceDialog {
    pub busy: bool,
    pub error: Option<String>,
}

pub enum Dialog {
    Profile(Box<ProfileForm>),
    Import(ImportForm),
    Share(ShareView),
    Rules(RulesForm),
    Subscription(SubscriptionForm),
    Value(ValueForm),
    Confirm(ConfirmDialog),
    Service(ServiceDialog),
}

pub fn kind_label(kind: RuleKind) -> &'static str {
    match kind {
        RuleKind::Proxy => "Always use tunnel",
        RuleKind::Direct => "Bypass tunnel",
        RuleKind::Block => "Block domains",
    }
}

pub fn kind_value(kind: RuleKind) -> &'static str {
    match kind {
        RuleKind::Proxy => "proxy",
        RuleKind::Direct => "direct",
        RuleKind::Block => "block",
    }
}

pub fn kind_from(value: &str) -> RuleKind {
    match value {
        "proxy" => RuleKind::Proxy,
        "direct" => RuleKind::Direct,
        _ => RuleKind::Block,
    }
}

pub fn rule_count(text: &str) -> usize {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .count()
}

fn is_hex_secret(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn check_url(url: &str) -> Result<(), String> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| "The link must use HTTP or HTTPS.".to_owned())?;
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    if host.is_empty()
        || host.starts_with(':')
        || url.len() > 2048
        || url.contains(char::is_whitespace)
    {
        return Err("Enter a valid URL.".to_owned());
    }
    Ok(())
}

fn parse_port(value: &str) -> Option<u16> {
    value.trim().parse::<u16>().ok().filter(|port| *port > 0)
}

pub const TEMPLATE_CURRENT: &str = "current";
pub const TEMPLATE_FILE: &str = "file";

pub fn template_options() -> Vec<(String, String, String)> {
    fiend_app::bundled_templates()
        .into_iter()
        .map(|template| {
            (
                template.id,
                format!("{} · {}", template.name, template.domain),
                template.template_json,
            )
        })
        .collect()
}

pub fn template_label(template_json: &str) -> String {
    template_options()
        .into_iter()
        .find(|template| template.2 == template_json)
        .map(|template| template.1)
        .unwrap_or_else(|| match fiend_app::template_domain(template_json) {
            Some(domain) => format!("Imported · {domain}"),
            None => "Imported template".to_owned(),
        })
}

impl AppView {
    fn new_input(
        &self,
        value: &str,
        placeholder: &str,
        masked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        let value = value.to_owned();
        let placeholder = placeholder.to_owned();
        let state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .masked(masked)
                .default_value(value)
        });
        cx.subscribe_in(
            &state,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.submit_dialog(window, cx),
                InputEvent::Change | InputEvent::Focus | InputEvent::Blur => {
                    this.on_dialog_input_change(cx);
                    cx.notify();
                }
            },
        )
        .detach();
        state
    }

    fn new_textarea(
        &self,
        value: &str,
        placeholder: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        let value = value.to_owned();
        let placeholder = placeholder.to_owned();
        let state = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(placeholder)
                .default_value(value)
        });
        cx.subscribe_in(&state, window, |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.on_dialog_input_change(cx);
            }
            cx.notify();
        })
        .detach();
        state
    }

    pub fn on_dialog_input_change(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::Rules(form)) = self.dialog.as_mut() else {
            return;
        };
        let text = form.text.read(cx).value().to_string();
        form.count = rule_count(&text);
        form.error = fiend_app::validate_rules(&text, form.kind != RuleKind::Block)
            .err()
            .map(|error| error.to_string());
    }

    pub fn open_dialog_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.dialog_focus.clone();
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
    }

    pub fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = None;
        self.menu = None;
        let focus = self.focus.clone();
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    fn profile_values(form: &ProfileForm, cx: &Context<Self>) -> String {
        let mut values = vec![
            form.name.read(cx).value().to_string(),
            form.host.read(cx).value().to_string(),
            form.port.read(cx).value().to_string(),
            form.secret.read(cx).value().to_string(),
            form.template_json.clone(),
        ];
        values.extend(
            form.tuning
                .iter()
                .map(|input| input.read(cx).value().to_string()),
        );
        values.join("\u{1f}")
    }

    pub fn request_close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.dialog.as_ref() {
            Some(Dialog::Profile(form)) if Self::profile_values(form, cx) != form.baseline => {
                self.open_confirm(
                    "Discard changes?",
                    "The changes to this profile are not saved.",
                    "Discard changes",
                    "Keep editing",
                    true,
                    ConfirmAction::DiscardProfile,
                    window,
                    cx,
                );
            }
            Some(Dialog::Rules(form)) if form.text.read(cx).value().as_ref() != form.baseline => {
                self.open_confirm(
                    "Discard changes?",
                    "The changes to this rule list are not saved.",
                    "Discard changes",
                    "Keep editing",
                    true,
                    ConfirmAction::DiscardRules,
                    window,
                    cx,
                );
            }
            Some(Dialog::Confirm(confirm)) => {
                let action = confirm.action.clone();
                self.cancel_confirm(action, window, cx);
            }
            Some(_) => self.close_dialog(window, cx),
            None => {}
        }
    }

    fn cancel_confirm(
        &mut self,
        action: ConfirmAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            ConfirmAction::DiscardProfile | ConfirmAction::DiscardRules => {
                if let Some(previous) = self.suspended_dialog.take() {
                    self.dialog = Some(previous);
                    self.open_dialog_focus(window, cx);
                    cx.notify();
                } else {
                    self.close_dialog(window, cx);
                }
            }
            _ => self.close_dialog(window, cx),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn open_confirm(
        &mut self,
        title: &str,
        body: &str,
        confirm: &str,
        cancel: &str,
        danger: bool,
        action: ConfirmAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let discard = matches!(
            action,
            ConfirmAction::DiscardProfile | ConfirmAction::DiscardRules
        );
        if discard {
            self.suspended_dialog = self.dialog.take();
        }
        self.menu = None;
        self.dialog = Some(Dialog::Confirm(ConfirmDialog {
            title: title.to_owned(),
            body: body.to_owned(),
            confirm: confirm.to_owned(),
            cancel: cancel.to_owned(),
            danger,
            action,
            busy: false,
        }));
        self.open_dialog_focus(window, cx);
        cx.notify();
    }

    pub fn run_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Dialog::Confirm(confirm)) = self.dialog.as_mut() else {
            return;
        };
        if confirm.busy {
            return;
        }
        let action = confirm.action.clone();
        match action {
            ConfirmAction::DiscardProfile | ConfirmAction::DiscardRules => {
                self.suspended_dialog = None;
                self.close_dialog(window, cx);
            }
            ConfirmAction::DeleteProfile(id) => {
                confirm.busy = true;
                self.call(Request::DeleteProfile { id }, cx, |this, result, cx| {
                    match result {
                        Ok(_) => this.show_toast("Profile deleted", false, cx),
                        Err(error) => this.show_toast(error, true, cx),
                    }
                    this.dialog = None;
                });
            }
            ConfirmAction::RemoveSubscription(id) => {
                confirm.busy = true;
                self.call(
                    Request::RemoveSubscription { id },
                    cx,
                    |this, result, cx| {
                        match result {
                            Ok(_) => this.show_toast("Subscription removed", false, cx),
                            Err(error) => this.show_toast(error, true, cx),
                        }
                        this.dialog = None;
                    },
                );
            }
            ConfirmAction::UninstallService => {
                confirm.busy = true;
                self.run_service_task(false, cx);
            }
        }
    }

    pub fn open_service_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = Some(Dialog::Service(ServiceDialog {
            busy: false,
            error: None,
        }));
        self.open_dialog_focus(window, cx);
        cx.notify();
    }

    pub fn run_service_task(&mut self, install: bool, cx: &mut Context<Self>) {
        if let Some(Dialog::Service(dialog)) = self.dialog.as_mut() {
            dialog.busy = true;
            dialog.error = None;
        }
        let task = cx.background_spawn(async move {
            if install {
                crate::service::install()
            } else {
                crate::service::uninstall(false)
            }
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.dialog = None;
                        this.show_toast(
                            if install {
                                "Service installed"
                            } else {
                                "Service removed"
                            },
                            false,
                            cx,
                        );
                    }
                    Err(error) => match this.dialog.as_mut() {
                        Some(Dialog::Service(dialog)) => {
                            dialog.busy = false;
                            dialog.error = Some(error);
                        }
                        _ => this.show_toast(error, true, cx),
                    },
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn open_profile_form(
        &mut self,
        id: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = id.as_deref().and_then(|id| self.model.profile(id)).cloned();
        let templates = template_options();
        let (name, host, port, secret, tuning, template_json) = match &existing {
            Some(profile) => {
                let (host, port) = format::host_port(&profile.server);
                (
                    profile.name.clone(),
                    host,
                    port,
                    profile.secret.clone(),
                    profile.tuning,
                    profile.template_json.clone(),
                )
            }
            None => (
                String::new(),
                String::new(),
                "443".to_owned(),
                String::new(),
                Tuning::default(),
                templates.first().map(|t| t.2.clone()).unwrap_or_default(),
            ),
        };
        let template = templates
            .iter()
            .find(|template| template.2 == template_json)
            .map(|template| template.0.clone())
            .unwrap_or_else(|| TEMPLATE_CURRENT.to_owned());
        let form = ProfileForm {
            id,
            name: self.new_input(&name, "Home server", false, window, cx),
            host: self.new_input(&host, "vpn.example.com", false, window, cx),
            port: self.new_input(&port, "443", false, window, cx),
            secret: self.new_input(&secret, "", true, window, cx),
            tuning: [
                self.new_input(
                    &tuning.handshake_timeout_ms.to_string(),
                    "",
                    false,
                    window,
                    cx,
                ),
                self.new_input(&tuning.max_connections.to_string(), "", false, window, cx),
                self.new_input(&tuning.udp_idle_ms.to_string(), "", false, window, cx),
                self.new_input(&tuning.udp_keepalive_ms.to_string(), "", false, window, cx),
                self.new_input(&tuning.max_udp_flows.to_string(), "", false, window, cx),
            ],
            template,
            template_json,
            errors: HashMap::new(),
            show_secret: false,
            tuning_open: false,
            baseline: String::new(),
            saving: false,
            form_error: None,
        };
        let baseline = Self::profile_values(&form, cx);
        let mut form = form;
        form.baseline = baseline;
        let first = form.name.clone();
        self.dialog = Some(Dialog::Profile(Box::new(form)));
        self.menu = None;
        let focus = first.read(cx).focus_handle(cx);
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    pub fn open_import(
        &mut self,
        link: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = link.unwrap_or_default();
        let state = self.new_textarea(&text, "fiend://v1/…", window, cx);
        let focus = state.read(cx).focus_handle(cx);
        self.dialog = Some(Dialog::Import(ImportForm {
            link: state,
            error: None,
            busy: false,
        }));
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    pub fn open_share(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(profile) = self.model.profile(id) else {
            return;
        };
        let name = profile.name.clone();
        self.call(
            Request::ShareLink { id: id.to_owned() },
            cx,
            move |this, result, cx| match result {
                Ok(Reply::Link { link }) => {
                    let (qr, note) = match qrcode::QrCode::with_error_correction_level(
                        link.as_bytes(),
                        qrcode::EcLevel::L,
                    ) {
                        Ok(code) => {
                            let width = code.width();
                            let modules = code
                                .to_colors()
                                .into_iter()
                                .map(|color| color == qrcode::Color::Dark)
                                .collect();
                            (Some((width, modules)), None)
                        }
                        Err(_) => (
                            None,
                            Some(
                                "This link is too large for a QR code. Copy the link instead."
                                    .to_owned(),
                            ),
                        ),
                    };
                    this.dialog = Some(Dialog::Share(ShareView {
                        name,
                        link,
                        qr,
                        note,
                    }));
                }
                Ok(_) => {}
                Err(error) => this.show_toast(error, true, cx),
            },
        );
        self.open_dialog_focus(window, cx);
    }

    pub fn open_rules(&mut self, kind: RuleKind, window: &mut Window, cx: &mut Context<Self>) {
        let text = match kind {
            RuleKind::Proxy => self.model.settings.proxy_rules.clone(),
            RuleKind::Direct => self.model.settings.direct_rules.clone(),
            RuleKind::Block => self.model.settings.block_rules.clone(),
        };
        let state = self.new_textarea(&text, "", window, cx);
        let focus = state.read(cx).focus_handle(cx);
        self.dialog = Some(Dialog::Rules(RulesForm {
            kind,
            count: rule_count(&text),
            baseline: text,
            text: state,
            error: None,
            busy: false,
        }));
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    pub fn open_subscription(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.new_input("", "https://example.com/rules.txt", false, window, cx);
        let focus = url.read(cx).focus_handle(cx);
        self.dialog = Some(Dialog::Subscription(SubscriptionForm {
            url,
            kind: "block".into(),
            error: None,
            busy: false,
        }));
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    pub fn open_value(&mut self, kind: ValueKind, window: &mut Window, cx: &mut Context<Self>) {
        let (value, placeholder) = match kind {
            ValueKind::Resolver => (self.model.settings.direct_resolver.clone(), "1.1.1.1:53"),
            ValueKind::Mtu => (self.model.settings.mtu.to_string(), "1500"),
        };
        let input = self.new_input(&value, placeholder, false, window, cx);
        let focus = input.read(cx).focus_handle(cx);
        self.dialog = Some(Dialog::Value(ValueForm {
            kind,
            input,
            error: None,
        }));
        window.defer(cx, move |window, cx| window.focus(&focus, cx));
        cx.notify();
    }

    pub fn apply_dialog_menu(
        &mut self,
        id: MenuId,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match (id, self.dialog.as_mut()) {
            (MenuId::Template, Some(Dialog::Profile(_))) if value == TEMPLATE_FILE => {
                self.load_template_file(window, cx);
            }
            (MenuId::Template, Some(Dialog::Profile(form))) => {
                if let Some(template) = template_options().into_iter().find(|t| t.0 == value) {
                    form.template = template.0;
                    form.template_json = template.2;
                    form.errors.remove("template");
                }
            }
            (MenuId::SubscriptionKind, Some(Dialog::Subscription(form))) => {
                form.kind = value.to_owned();
            }
            _ => {}
        }
        cx.notify();
    }

    pub fn submit_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.dialog.as_ref() {
            Some(Dialog::Profile(_)) => self.submit_profile(window, cx),
            Some(Dialog::Import(_)) => self.submit_import(window, cx),
            Some(Dialog::Rules(_)) => self.submit_rules(cx),
            Some(Dialog::Subscription(_)) => self.submit_subscription(cx),
            Some(Dialog::Value(_)) => self.submit_value(window, cx),
            Some(Dialog::Confirm(_)) => self.run_confirm(window, cx),
            Some(Dialog::Service(_)) => self.run_service_task(true, cx),
            _ => {}
        }
    }

    fn submit_profile(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(Dialog::Profile(form)) = self.dialog.as_mut() else {
            return;
        };
        if form.saving {
            return;
        }
        let read = |entity: &Entity<InputState>| entity.read(cx).value().to_string();
        let name = read(&form.name).trim().to_owned();
        let host = read(&form.host).trim().to_owned();
        let port = read(&form.port);
        let secret = read(&form.secret).trim().to_owned();
        let values: Vec<String> = form.tuning.iter().map(read).collect();
        let mut errors: HashMap<&'static str, String> = HashMap::new();
        if name.is_empty() {
            errors.insert("name", "This field is required.".into());
        } else if name.len() > 128 {
            errors.insert("name", "Use 128 bytes or fewer.".into());
        }
        if host.is_empty() {
            errors.insert("host", "This field is required.".into());
        }
        if parse_port(&port).is_none() {
            errors.insert("port", "Enter a port from 1 to 65535.".into());
        }
        if !is_hex_secret(&secret) {
            errors.insert("secret", "Enter 64 hexadecimal characters.".into());
        }
        let numbers: Vec<Option<u32>> = values.iter().map(|v| v.trim().parse().ok()).collect();
        let tuning = match numbers.as_slice() {
            [Some(a), Some(b), Some(c), Some(d), Some(e)] => Some(Tuning {
                handshake_timeout_ms: *a,
                max_connections: *b,
                udp_idle_ms: *c,
                udp_keepalive_ms: *d,
                max_udp_flows: *e,
            }),
            _ => {
                errors.insert("tuning", "Enter whole numbers.".into());
                None
            }
        };
        if let Some(tuning) = &tuning
            && let Err(reason) = tuning.validate()
        {
            errors.insert("tuning", reason.to_owned());
        }
        form.form_error = None;
        if !errors.is_empty() {
            if errors.contains_key("tuning") {
                form.tuning_open = true;
            }
            form.errors = errors;
            cx.notify();
            return;
        }
        form.errors.clear();
        form.saving = true;
        let template_json = if form.template == TEMPLATE_CURRENT {
            form.template_json.clone()
        } else {
            template_options()
                .into_iter()
                .find(|template| template.0 == form.template)
                .map(|template| template.2)
                .unwrap_or_else(|| form.template_json.clone())
        };
        let input = ProfileInput {
            id: form.id.clone(),
            name,
            server: format::join_host_port(&host, port.trim()),
            secret,
            template_json,
            tuning: tuning.unwrap_or_default(),
        };
        self.call(
            Request::SaveProfile { profile: input },
            cx,
            |this, result, cx| match result {
                Ok(Reply::Profile(profile)) => {
                    this.selected_profile = Some(profile.id);
                    this.dialog = None;
                    this.show_toast("Profile saved", false, cx);
                }
                Ok(_) => this.dialog = None,
                Err(error) => {
                    if let Some(Dialog::Profile(form)) = this.dialog.as_mut() {
                        form.saving = false;
                        form.form_error = Some(error);
                    }
                }
            },
        );
    }

    fn submit_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Dialog::Import(form)) = self.dialog.as_mut() else {
            return;
        };
        if form.busy {
            return;
        }
        let link = form.link.read(cx).value().trim().to_owned();
        if link.is_empty() {
            form.error = Some("Paste a share link.".into());
            cx.notify();
            return;
        }
        form.busy = true;
        form.error = None;
        self.call(
            Request::ImportLink { link },
            cx,
            |this, result, cx| match result {
                Ok(Reply::Profile(profile)) => {
                    this.selected_profile = Some(profile.id);
                    this.dialog = None;
                    this.show_toast("Profile imported", false, cx);
                }
                Ok(_) => this.dialog = None,
                Err(error) => {
                    if let Some(Dialog::Import(form)) = this.dialog.as_mut() {
                        form.busy = false;
                        form.error = Some(error);
                    }
                }
            },
        );
        let _ = window;
    }

    fn submit_rules(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::Rules(form)) = self.dialog.as_mut() else {
            return;
        };
        if form.busy || form.error.is_some() {
            return;
        }
        form.busy = true;
        let kind = form.kind;
        let text = form.text.read(cx).value().to_string();
        self.call(
            Request::SetRuleText { kind, text },
            cx,
            |this, result, cx| match result {
                Ok(_) => {
                    this.dialog = None;
                    let message = if this.model.status.restart_required {
                        "Rules saved. Reconnect to apply them."
                    } else {
                        "Rules saved"
                    };
                    this.show_toast(message, false, cx);
                }
                Err(error) => {
                    if let Some(Dialog::Rules(form)) = this.dialog.as_mut() {
                        form.busy = false;
                        form.error = Some(error);
                    }
                }
            },
        );
    }

    fn submit_subscription(&mut self, cx: &mut Context<Self>) {
        let Some(Dialog::Subscription(form)) = self.dialog.as_mut() else {
            return;
        };
        if form.busy {
            return;
        }
        let url = form.url.read(cx).value().trim().to_owned();
        if let Err(error) = check_url(&url) {
            form.error = Some(error);
            cx.notify();
            return;
        }
        form.busy = true;
        form.error = None;
        let kind = kind_from(&form.kind);
        self.call(
            Request::AddSubscription { url, kind },
            cx,
            |this, result, cx| match result {
                Ok(Reply::Subscription(subscription)) => {
                    this.dialog = None;
                    match subscription.error {
                        Some(error) => this.show_toast(
                            format!("Added, but the download failed: {error}"),
                            true,
                            cx,
                        ),
                        None => this.show_toast("Subscription added", false, cx),
                    }
                }
                Ok(_) => this.dialog = None,
                Err(error) => {
                    if let Some(Dialog::Subscription(form)) = this.dialog.as_mut() {
                        form.busy = false;
                        form.error = Some(error);
                    }
                }
            },
        );
    }

    fn submit_value(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(Dialog::Value(form)) = self.dialog.as_mut() else {
            return;
        };
        let text = form.input.read(cx).value().trim().to_owned();
        let mut settings: Settings = self.model.settings.clone();
        match form.kind {
            ValueKind::Resolver => {
                if let Err(error) = fiend_app::validate_resolver(&text) {
                    form.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
                settings.direct_resolver = text;
            }
            ValueKind::Mtu => match text.parse::<u16>() {
                Ok(mtu) if (fiend_ipc::MIN_MTU..=fiend_ipc::MAX_MTU).contains(&mtu) => {
                    settings.mtu = mtu;
                }
                _ => {
                    form.error = Some(format!(
                        "Enter a number from {} to {}.",
                        fiend_ipc::MIN_MTU,
                        fiend_ipc::MAX_MTU
                    ));
                    cx.notify();
                    return;
                }
            },
        }
        self.call(
            Request::SetSettings { settings },
            cx,
            |this, result, cx| match result {
                Ok(_) => {
                    this.dialog = None;
                    this.show_toast("Saved", false, cx);
                }
                Err(error) => {
                    if let Some(Dialog::Value(form)) = this.dialog.as_mut() {
                        form.error = Some(error);
                    }
                }
            },
        );
    }

    pub fn scan_screen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let task = cx.background_spawn(async { crate::scan::scan_screen() });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(link) => this.load_text(link.as_bytes(), false, window, cx),
                Err(error) => {
                    if let Some(Dialog::Import(form)) = this.dialog.as_mut() {
                        form.error = Some(error);
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub fn load_template_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let Some(file) = rfd::AsyncFileDialog::new()
                .set_title("Load a TLS template")
                .pick_file()
                .await
            else {
                return;
            };
            let bytes = file.read().await;
            let _ = this.update(cx, |this, cx| {
                if let Some(Dialog::Profile(form)) = this.dialog.as_mut() {
                    match fiend_app::load_template(&bytes) {
                        Ok(loaded) => {
                            form.template = template_options()
                                .into_iter()
                                .find(|template| template.2 == loaded.template_json)
                                .map(|template| template.0)
                                .unwrap_or_else(|| TEMPLATE_CURRENT.to_owned());
                            form.template_json = loaded.template_json;
                            form.errors.remove("template");
                        }
                        Err(error) => {
                            form.errors.insert("template", error.to_string());
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn import_file(&mut self, rules: bool, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            let Some(file) = rfd::AsyncFileDialog::new().pick_file().await else {
                return;
            };
            let bytes = file.read().await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.load_text(&bytes, rules, window, cx);
            });
        })
        .detach();
    }

    pub fn load_text(
        &mut self,
        bytes: &[u8],
        rules: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Ok(text) = std::str::from_utf8(bytes) else {
            self.show_toast("The file is not a text file", true, cx);
            return;
        };
        if bytes.len() > 16 << 20 {
            self.show_toast("The file is too large", true, cx);
            return;
        }
        match self.dialog.as_ref() {
            Some(Dialog::Import(form)) if !rules => {
                let state = form.link.clone();
                let text = text.trim().to_owned();
                state.update(cx, |state, cx| state.set_value(text, window, cx));
            }
            Some(Dialog::Rules(form)) if rules => {
                let state = form.text.clone();
                let text = text.to_owned();
                state.update(cx, |state, cx| state.set_value(text, window, cx));
                self.on_dialog_input_change(cx);
            }
            _ => {}
        }
        cx.notify();
    }

    pub fn dropped_files(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = paths.paths().first() else {
            return;
        };
        let rules = matches!(self.dialog, Some(Dialog::Rules(_)));
        match std::fs::read(path) {
            Ok(bytes) => self.load_text(&bytes, rules, window, cx),
            Err(error) => self.show_toast(error.to_string(), true, cx),
        }
    }

    fn input_box(
        &self,
        ui: &Ui,
        state: &Entity<InputState>,
        window: &Window,
        cx: &Context<Self>,
        invalid: bool,
        mono: bool,
    ) -> gpui_kit::Div {
        let p = ui.p;
        let focused = state.read(cx).focus_handle(cx).is_focused(window);
        div()
            .w_full()
            .h(px(36.))
            .px(px(10.))
            .flex()
            .items_center()
            .rounded(px(4.))
            .bg(p.panel)
            .border_1()
            .border_color(if invalid {
                p.danger
            } else if focused {
                p.accent
            } else {
                p.line
            })
            .text_size(px(12.))
            .text_color(p.text)
            .when(mono, |this| this.font_family(MONO))
            .child(div().w_full().child(Input::new(state)))
    }

    fn field(
        &self,
        ui: &Ui,
        label: &str,
        input: gpui_kit::Div,
        error: Option<&String>,
        help: Option<&str>,
    ) -> gpui_kit::Div {
        div()
            .mb(px(14.))
            .min_w_0()
            .child(ui.field_label(label))
            .child(input)
            .when_some(error, |this, error| this.child(ui.field_error(error)))
            .when_some(help, |this, help| {
                this.child(
                    div()
                        .mt(px(5.))
                        .text_size(px(10.))
                        .text_color(ui.p.muted)
                        .child(help.to_owned()),
                )
            })
    }

    #[allow(clippy::too_many_arguments)]
    fn modal(
        &mut self,
        ui: &Ui,
        title: &str,
        description: Option<&str>,
        content: gpui_kit::AnyElement,
        actions: gpui_kit::AnyElement,
        wide: bool,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let p = ui.p;
        let panel = div()
            .id("dialog-panel")
            .track_focus(&self.dialog_focus)
            .key_context("Dialog")
            .on_key_down(
                cx.listener(|this, event: &gpui_kit::KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        this.request_close_dialog(window, cx);
                        cx.stop_propagation();
                    }
                }),
            )
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                this.dropped_files(paths, window, cx);
            }))
            .occlude()
            .w(px(if wide { 660. } else { 470. }))
            .max_h(px(620.))
            .flex()
            .flex_col()
            .rounded(px(7.))
            .bg(p.surface)
            .border_1()
            .border_color(p.line)
            .shadow_lg()
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap(px(15.))
                    .px(px(24.))
                    .pt(px(22.))
                    .pb(px(15.))
                    .child(
                        div()
                            .min_w_0()
                            .child(
                                div()
                                    .text_size(px(19.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title.to_owned()),
                            )
                            .when_some(description, |this, text| {
                                this.child(
                                    div()
                                        .mt(px(5.))
                                        .text_size(px(11.))
                                        .text_color(p.muted)
                                        .child(text.to_owned()),
                                )
                            }),
                    )
                    .child(
                        ui.icon_button("dialog-close", "close", "Close dialog")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.request_close_dialog(window, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .id("dialog-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(24.))
                    .pb(px(22.))
                    .child(content),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(8.))
                    .px(px(24.))
                    .py(px(16.))
                    .border_t_1()
                    .border_color(p.line)
                    .child(actions),
            )
            .focus_trap("dialog-trap", &self.dialog_focus);
        div()
            .id("dialog-overlay")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(p.scrim)
            .occlude()
            .child(panel)
            .into_any_element()
    }

    pub fn render_dialog(
        &mut self,
        ui: &Ui,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui_kit::AnyElement> {
        let dialog = self.dialog.take()?;
        let element = self.render_dialog_inner(ui, &dialog, window, cx);
        self.dialog = Some(dialog);
        Some(element)
    }

    fn actions(
        &self,
        ui: &Ui,
        left: Option<gpui_kit::AnyElement>,
        cancel: &str,
        primary: Option<(&str, ButtonKind, bool)>,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let mut row = div()
            .flex()
            .items_center()
            .justify_end()
            .gap(px(8.))
            .w_full();
        if let Some(left) = left {
            row = row.child(div().mr_auto().child(left));
        }
        row = row.child(
            ui.button("dialog-cancel", cancel, None, ButtonKind::Ghost)
                .on_click(cx.listener(|this, _, window, cx| this.request_close_dialog(window, cx))),
        );
        if let Some((label, kind, busy)) = primary {
            row = row.child(
                ui.button("dialog-primary", label, None, kind)
                    .disabled(busy)
                    .on_click(cx.listener(|this, _, window, cx| this.submit_dialog(window, cx))),
            );
        }
        row.into_any_element()
    }

    fn render_dialog_inner(
        &mut self,
        ui: &Ui,
        dialog: &Dialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui_kit::AnyElement {
        let p = ui.p;
        match dialog {
            Dialog::Profile(form) => {
                let templates = template_options();
                let mut options: Vec<MenuOption> = templates
                    .iter()
                    .map(|t| MenuOption::new(t.0.clone(), t.1.clone()))
                    .collect();
                if form.template == TEMPLATE_CURRENT {
                    options.insert(
                        0,
                        MenuOption::new(TEMPLATE_CURRENT, template_label(&form.template_json)),
                    );
                }
                options.push(MenuOption::new(TEMPLATE_FILE, "Load from file…"));
                let label = options
                    .iter()
                    .find(|option| option.value == form.template)
                    .map(|option| option.label.clone())
                    .unwrap_or_default();
                let err = |key: &str| form.errors.get(key);
                let secret_box =
                    self.input_box(ui, &form.secret, window, cx, err("secret").is_some(), true);
                let secret_toggle = ui
                    .icon_button(
                        "secret-toggle",
                        "eye",
                        if form.show_secret {
                            "Hide secret"
                        } else {
                            "Show secret"
                        },
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(Dialog::Profile(form)) = this.dialog.as_mut() {
                            form.show_secret = !form.show_secret;
                            let show = form.show_secret;
                            form.secret
                                .update(cx, |state, cx| state.set_masked(!show, window, cx));
                        }
                        cx.notify();
                    }));
                let template = self.select(
                    ui,
                    MenuId::Template,
                    label,
                    options,
                    form.template.clone(),
                    false,
                    410.,
                    cx,
                );
                let tuning_labels = [
                    "Handshake timeout (ms)",
                    "Max connections",
                    "UDP idle timeout (ms)",
                    "UDP keepalive (ms)",
                    "Max UDP flows",
                ];
                let mut grid = div().mt(px(10.)).grid().grid_cols(2).gap(px(12.));
                for (index, label) in tuning_labels.iter().enumerate() {
                    let input = self.input_box(ui, &form.tuning[index], window, cx, false, false);
                    grid = grid.child(self.field(ui, label, input, None, None));
                }
                let tuning_open = form.tuning_open;
                let name_box =
                    self.input_box(ui, &form.name, window, cx, err("name").is_some(), false);
                let host_box =
                    self.input_box(ui, &form.host, window, cx, err("host").is_some(), false);
                let port_box =
                    self.input_box(ui, &form.port, window, cx, err("port").is_some(), true);
                let content = div()
                    .child(self.field(ui, "Profile name", name_box, err("name"), None))
                    .child(
                        div()
                            .flex()
                            .gap(px(12.))
                            .child(div().flex_1().min_w_0().child(self.field(
                                ui,
                                "Host",
                                host_box,
                                err("host"),
                                None,
                            )))
                            .child(div().w(px(90.)).child(self.field(
                                ui,
                                "Port",
                                port_box,
                                err("port"),
                                None,
                            ))),
                    )
                    .child(
                        self.field(
                            ui,
                            "Secret",
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(div().flex_1().child(secret_box))
                                .child(secret_toggle),
                            err("secret"),
                            Some("64 hexadecimal characters."),
                        ),
                    )
                    .child(self.field(
                        ui,
                        "TLS fingerprint",
                        template,
                        err("template"),
                        Some("A template JSON or a ClientHello capture of a real site."),
                    ))
                    .child(
                        div()
                            .mt(px(6.))
                            .child(
                                ui.button(
                                    "tuning-toggle",
                                    "Connection tuning",
                                    Some(if tuning_open { "down" } else { "chevron" }),
                                    ButtonKind::Ghost,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        if let Some(Dialog::Profile(form)) = this.dialog.as_mut() {
                                            form.tuning_open = !form.tuning_open;
                                        }
                                        cx.notify();
                                    },
                                )),
                            )
                            .when(tuning_open, |this| this.child(grid))
                            .when_some(err("tuning"), |this, error| {
                                this.child(ui.field_error(error))
                            }),
                    )
                    .when_some(form.form_error.clone(), |this, error| {
                        this.child(
                            div()
                                .mt(px(10.))
                                .text_size(px(11.))
                                .text_color(p.danger)
                                .child(error),
                        )
                    });
                let actions = self.actions(
                    ui,
                    None,
                    "Cancel",
                    Some((
                        if form.id.is_some() {
                            "Save profile"
                        } else {
                            "Create profile"
                        },
                        ButtonKind::Primary,
                        form.saving,
                    )),
                    cx,
                );
                self.modal(
                    ui,
                    if form.id.is_some() {
                        "Edit profile"
                    } else {
                        "New profile"
                    },
                    None,
                    content.into_any_element(),
                    actions,
                    false,
                    cx,
                )
            }
            Dialog::Import(form) => {
                let focused = form.link.read(cx).focus_handle(cx).is_focused(window);
                let content = div()
                    .child(ui.field_label("Share link"))
                    .child(
                        div()
                            .h(px(110.))
                            .p(px(10.))
                            .rounded(px(4.))
                            .bg(p.panel)
                            .border_1()
                            .border_color(if focused { p.accent } else { p.line })
                            .font_family(MONO)
                            .text_size(px(11.))
                            .child(Textarea::new(&form.link)),
                    )
                    .child(
                        div()
                            .mt(px(12.))
                            .flex()
                            .gap(px(8.))
                            .child(
                                ui.button(
                                    "import-file",
                                    "Choose file",
                                    Some("file"),
                                    ButtonKind::Default,
                                )
                                .on_click(cx.listener(
                                    |this, _, window, cx| this.import_file(false, window, cx),
                                )),
                            )
                            .child(
                                ui.button(
                                    "import-scan",
                                    "Scan screen",
                                    Some("eye"),
                                    ButtonKind::Ghost,
                                )
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.scan_screen(window, cx)),
                                ),
                            )
                            .child(
                                ui.button("import-paste", "Paste", None, ButtonKind::Ghost)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(text) =
                                            cx.read_from_clipboard().and_then(|item| item.text())
                                        {
                                            this.load_text(text.as_bytes(), false, window, cx);
                                        }
                                    })),
                            ),
                    )
                    .when_some(form.error.clone(), |this, error| {
                        this.child(
                            div()
                                .mt(px(10.))
                                .text_size(px(11.))
                                .text_color(p.danger)
                                .child(error),
                        )
                    });
                let actions = self.actions(
                    ui,
                    None,
                    "Cancel",
                    Some(("Import profile", ButtonKind::Primary, form.busy)),
                    cx,
                );
                self.modal(
                    ui,
                    "Import profile",
                    Some("Paste a fiend:// link, drop a file here, or choose a file."),
                    content.into_any_element(),
                    actions,
                    false,
                    cx,
                )
            }
            Dialog::Share(share) => {
                let link = share.link.clone();
                let content = div()
                    .child(ui.notice(
                        "info",
                        "Contains the server secret. Share it only with people you trust.",
                        false,
                    ))
                    .when_some(share.qr.clone(), |this, (width, modules)| {
                        let dark = p.text;
                        let light = p.panel;
                        this.child(
                            div().mb(px(14.)).flex().justify_center().child(
                                div()
                                    .p(px(10.))
                                    .rounded(px(4.))
                                    .bg(light)
                                    .border_1()
                                    .border_color(p.line)
                                    .child(
                                        canvas(
                                            |_, _, _| (),
                                            move |bounds, _, window, _| {
                                                let cell = (bounds.size.width
                                                    / (width as f32 + 0.0))
                                                    .floor()
                                                    .max(px(1.));
                                                for (index, on) in modules.iter().enumerate() {
                                                    if *on {
                                                        let x = (index % width) as f32;
                                                        let y = (index / width) as f32;
                                                        window.paint_quad(fill(
                                                            gpui_kit::Bounds::new(
                                                                point(
                                                                    bounds.origin.x + cell * x,
                                                                    bounds.origin.y + cell * y,
                                                                ),
                                                                size(cell, cell),
                                                            ),
                                                            dark,
                                                        ));
                                                    }
                                                }
                                            },
                                        )
                                        .size(px(width as f32 * 4.))
                                        .min_w(px(width as f32 * 4.)),
                                    ),
                            ),
                        )
                    })
                    .when_some(share.note.clone(), |this, note| {
                        this.child(
                            div()
                                .mb(px(12.))
                                .text_size(px(11.))
                                .text_color(p.muted)
                                .child(note),
                        )
                    })
                    .child(ui.field_label("Share link"))
                    .child(
                        div()
                            .max_h(px(110.))
                            .overflow_hidden()
                            .p(px(10.))
                            .rounded(px(4.))
                            .bg(p.panel)
                            .border_1()
                            .border_color(p.line)
                            .font_family(MONO)
                            .text_size(px(10.))
                            .child(format!("{}…", link.chars().take(320).collect::<String>())),
                    );
                let copy_link = link.clone();
                let actions = div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        ui.button("dialog-cancel", "Close", None, ButtonKind::Ghost)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.close_dialog(window, cx)),
                            ),
                    )
                    .child(
                        ui.button("copy-link", "Copy link", Some("copy"), ButtonKind::Primary)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy_link.clone()));
                                this.show_toast("Link copied", false, cx);
                            })),
                    )
                    .into_any_element();
                self.modal(
                    ui,
                    &format!("Share {}", share.name),
                    None,
                    content.into_any_element(),
                    actions,
                    false,
                    cx,
                )
            }
            Dialog::Rules(form) => {
                let kind = form.kind;
                let focused = form.text.read(cx).focus_handle(cx).is_focused(window);
                let content = div()
                    .child(ui.field_label("Rule list"))
                    .child(
                        div()
                            .h(px(260.))
                            .p(px(15.))
                            .rounded(px(4.))
                            .bg(p.panel)
                            .border_1()
                            .border_color(if form.error.is_some() {
                                p.danger
                            } else if focused {
                                p.accent
                            } else {
                                p.line
                            })
                            .font_family(MONO)
                            .text_size(px(11.))
                            .child(Textarea::new(&form.text)),
                    )
                    .child(
                        div()
                            .mt(px(8.))
                            .mb(px(16.))
                            .flex()
                            .justify_between()
                            .text_size(px(10.))
                            .text_color(p.muted)
                            .child(format!("{} rules", form.count))
                            .child(if form.error.is_some() {
                                "Fix the error to save"
                            } else {
                                "Valid"
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(p.muted)
                            .line_height(px(19.))
                            .child("domain: Domain and subdomains   full: Exact domain")
                            .child(
                                "keyword: Part of a domain".to_owned()
                                    + if kind != RuleKind::Block {
                                        "   ip: Address range"
                                    } else {
                                        ""
                                    },
                            )
                            .child(match kind {
                                RuleKind::Proxy => "# Comment. Tunnel rules win over bypass rules.",
                                RuleKind::Direct => {
                                    "# Comment. These sites see your own network address."
                                }
                                RuleKind::Block => "# Comment. Blocked domains do not resolve.",
                            }),
                    )
                    .when_some(form.error.clone(), |this, error| {
                        this.child(
                            div()
                                .mt(px(10.))
                                .text_size(px(11.))
                                .text_color(p.danger)
                                .child(error),
                        )
                    });
                let left = ui
                    .button("rules-file", "Import file", Some("file"), ButtonKind::Ghost)
                    .on_click(cx.listener(|this, _, window, cx| this.import_file(true, window, cx)))
                    .into_any_element();
                let actions = self.actions(
                    ui,
                    Some(left),
                    "Cancel",
                    Some((
                        "Save rules",
                        ButtonKind::Primary,
                        form.busy || form.error.is_some(),
                    )),
                    cx,
                );
                self.modal(
                    ui,
                    kind_label(kind),
                    Some("One rule per line. Changes apply after reconnect."),
                    content.into_any_element(),
                    actions,
                    true,
                    cx,
                )
            }
            Dialog::Subscription(form) => {
                let options = vec![
                    MenuOption::new("block", "Block domains"),
                    MenuOption::new("direct", "Bypass tunnel"),
                    MenuOption::new("proxy", "Always use tunnel"),
                ];
                let kind_select = self.select(
                    ui,
                    MenuId::SubscriptionKind,
                    kind_label(kind_from(&form.kind)).to_owned(),
                    options,
                    form.kind.clone(),
                    false,
                    410.,
                    cx,
                );
                let url_box =
                    self.input_box(ui, &form.url, window, cx, form.error.is_some(), false);
                let content = div()
                    .child(self.field(ui, "Rule list URL", url_box, form.error.as_ref(), None))
                    .child(self.field(ui, "List type", kind_select, None, None));
                let actions = self.actions(
                    ui,
                    None,
                    "Cancel",
                    Some(("Add subscription", ButtonKind::Primary, form.busy)),
                    cx,
                );
                self.modal(
                    ui,
                    "Add subscription",
                    Some("Use an HTTP or HTTPS link to a .srs or text rule list."),
                    content.into_any_element(),
                    actions,
                    false,
                    cx,
                )
            }
            Dialog::Value(form) => {
                let resolver = form.kind == ValueKind::Resolver;
                let input = self.input_box(ui, &form.input, window, cx, form.error.is_some(), true);
                let content = self.field(
                    ui,
                    if resolver { "Resolver address" } else { "MTU" },
                    input,
                    form.error.as_ref(),
                    Some(if resolver {
                        "IPv4:port, or [IPv6]:port. For example, 1.1.1.1:53. Leave empty for the default."
                    } else {
                        "1280 to 9000 bytes. Default: 1500."
                    }),
                );
                let actions = self.actions(
                    ui,
                    None,
                    "Cancel",
                    Some(("Save", ButtonKind::Primary, false)),
                    cx,
                );
                self.modal(
                    ui,
                    if resolver {
                        "Direct DNS resolver"
                    } else {
                        "Tunnel MTU"
                    },
                    Some(if resolver {
                        "Used only for traffic outside the tunnel."
                    } else {
                        "Reconnect to apply this setting."
                    }),
                    content.into_any_element(),
                    actions,
                    false,
                    cx,
                )
            }
            Dialog::Confirm(confirm) => {
                let content = div()
                    .text_size(px(12.))
                    .text_color(p.muted)
                    .child(confirm.body.clone());
                let actions = self.actions(
                    ui,
                    None,
                    &confirm.cancel,
                    Some((
                        &confirm.confirm,
                        if confirm.danger {
                            ButtonKind::Danger
                        } else {
                            ButtonKind::Primary
                        },
                        confirm.busy,
                    )),
                    cx,
                );
                self.modal(
                    ui,
                    &confirm.title,
                    None,
                    content.into_any_element(),
                    actions,
                    false,
                    cx,
                )
            }
            Dialog::Service(service) => {
                let name = if cfg!(target_os = "macos") {
                    "macOS LaunchDaemon"
                } else if cfg!(target_os = "windows") {
                    "Windows service"
                } else {
                    "Linux systemd service"
                };
                let content = div()
                    .child(ui.notice(
                        "info",
                        "Administrator access is required to create the tunnel and set routes and DNS. You enter the password one time.",
                        false,
                    ))
                    .child(ui.group().child(ui.setting_row(name, Some("Runs in the background."), div(), true)))
                    .when_some(service.error.clone(), |this, error| {
                        this.child(div().mt(px(12.)).text_size(px(11.)).text_color(p.danger).child(error))
                    });
                let actions = self.actions(
                    ui,
                    None,
                    "Not now",
                    Some((
                        if service.busy {
                            "Installing…"
                        } else {
                            "Install service"
                        },
                        ButtonKind::Primary,
                        service.busy,
                    )),
                    cx,
                );
                self.modal(
                    ui,
                    "Install system service",
                    None,
                    content.into_any_element(),
                    actions,
                    false,
                    cx,
                )
            }
        }
    }

    pub fn pick_profile(&mut self, id: String, cx: &mut Context<Self>) {
        if self.model.status.profile_id.as_deref() == Some(id.as_str())
            || (!matches!(
                self.model.status.state,
                fiend_ipc::TunnelState::Connected { .. }
                    | fiend_ipc::TunnelState::Connecting { .. }
            ) && self.model.settings.active_profile.as_deref() == Some(id.as_str()))
        {
            return;
        }
        if matches!(
            self.model.status.state,
            fiend_ipc::TunnelState::Connected { .. } | fiend_ipc::TunnelState::Connecting { .. }
        ) {
            self.connect(Some(id), cx);
        } else {
            let mut settings = self.model.settings.clone();
            settings.active_profile = Some(id);
            self.call_toast(Request::SetSettings { settings }, cx);
        }
    }

    pub fn go_to_profile(&mut self, id: &str, cx: &mut Context<Self>) {
        self.selected_profile = Some(id.to_owned());
        self.navigate(Page::Profiles, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_lines_ignore_comments_and_blanks() {
        assert_eq!(rule_count("domain:a.ru\n\n# note\n  full:b.ru  \n"), 2);
        assert_eq!(rule_count(""), 0);
    }

    #[test]
    fn secrets_and_ports_are_checked() {
        assert!(is_hex_secret(&"ab".repeat(32)));
        assert!(!is_hex_secret(&"zz".repeat(32)));
        assert!(!is_hex_secret("abcd"));
        assert_eq!(parse_port(" 443 "), Some(443));
        assert_eq!(parse_port("0"), None);
        assert_eq!(parse_port("70000"), None);
    }

    #[test]
    fn urls_are_checked_before_they_are_sent() {
        assert!(check_url("https://example.com/a.srs").is_ok());
        assert!(check_url("http://1.2.3.4:8080/x").is_ok());
        for bad in [
            "",
            "ftp://x/y",
            "https://",
            "https:// a",
            "example.com",
            "http://:80/x",
        ] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn rule_kind_names_round_trip() {
        for kind in [RuleKind::Proxy, RuleKind::Direct, RuleKind::Block] {
            assert_eq!(kind_from(kind_value(kind)), kind);
        }
    }
}
