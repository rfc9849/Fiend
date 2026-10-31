#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{Parser, Subcommand, ValueEnum};
use fiend_ipc::{
    Client, ClientError, Event, LogEntry, LogLevel, Profile, RuleKind, Settings, Stats, Status,
    TunnelState,
};
use tokio::io::AsyncReadExt;

macro_rules! println {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        if let Err(error) = writeln!(std::io::stdout(), $($arg)*) {
            stdout_failed(&error);
        }
    }};
}

fn stdout_failed(error: &std::io::Error) -> ! {
    if error.kind() == std::io::ErrorKind::BrokenPipe {
        std::process::exit(0);
    }
    eprintln!("fiend: cannot write the output: {error}");
    std::process::exit(1);
}

#[derive(Parser)]
#[command(name = "fiend", version, about = "Control the Fiend daemon")]
struct Cli {
    /// Daemon socket path.
    #[arg(long, global = true)]
    socket: Option<PathBuf>,
    /// Print JSON.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum Kind {
    Direct,
    Block,
    Proxy,
}

impl From<Kind> for RuleKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Direct => Self::Direct,
            Kind::Block => Self::Block,
            Kind::Proxy => Self::Proxy,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Show the tunnel state.
    Status,
    /// Connect. Uses the active profile when you give none.
    Connect {
        /// Profile ID, ID prefix, or exact name.
        profile: Option<String>,
        /// Return at once; do not wait for Connected.
        #[arg(long)]
        no_wait: bool,
    },
    /// Disconnect.
    Disconnect,
    /// Print state changes and statistics until you stop it.
    Watch,
    /// Saved profiles.
    #[command(subcommand)]
    Profiles(ProfileCommand),
    /// Show the daemon log.
    Logs {
        /// Number of lines.
        #[arg(short = 'n', long, default_value_t = 50)]
        lines: u32,
        /// Keep printing new lines.
        #[arg(short, long)]
        follow: bool,
        /// Clear the log.
        #[arg(long, conflicts_with = "follow")]
        clear: bool,
    },
    /// Show settings as JSON.
    Settings,
    /// Set the text of a rule list from a file or from standard input.
    Rules {
        kind: Kind,
        /// File to read. Use - for standard input.
        file: String,
    },
    /// Rule list subscriptions.
    #[command(subcommand)]
    Subscriptions(SubscriptionCommand),
}

#[derive(Subcommand)]
enum ProfileCommand {
    List,
    /// Import a fiend:// link. Use - to read it from standard input.
    Import {
        link: String,
    },
    /// Print the share link of a profile.
    Share {
        profile: String,
    },
    Delete {
        profile: String,
    },
    /// Measure the handshake time to the server.
    Latency {
        profile: String,
    },
}

#[derive(Subcommand)]
enum SubscriptionCommand {
    List,
    Add {
        url: String,
        #[arg(long, value_enum, default_value = "direct")]
        kind: Kind,
    },
    Remove {
        id: String,
    },
    Refresh,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("fiend: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("fiend: {message}");
            ExitCode::FAILURE
        }
    }
}

fn fail(error: ClientError) -> String {
    match error {
        ClientError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
            "the daemon is not running (socket not found)".into()
        }
        ClientError::Io(io) if io.kind() == std::io::ErrorKind::ConnectionRefused => {
            "the daemon is not running (connection refused)".into()
        }
        other => other.to_string(),
    }
}

async fn read_input(source: &str) -> Result<String, String> {
    if source == "-" {
        let mut text = String::new();
        tokio::io::stdin()
            .read_to_string(&mut text)
            .await
            .map_err(|error| error.to_string())?;
        Ok(text)
    } else {
        tokio::fs::read_to_string(source)
            .await
            .map_err(|error| format!("{source}: {error}"))
    }
}

fn find_profile<'a>(profiles: &'a [Profile], reference: &str) -> Result<&'a Profile, String> {
    let exact: Vec<_> = profiles
        .iter()
        .filter(|profile| profile.id == reference || profile.name == reference)
        .collect();
    let candidates = if exact.is_empty() {
        profiles
            .iter()
            .filter(|profile| profile.id.starts_with(reference))
            .collect()
    } else {
        exact
    };
    match candidates.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("no profile matches \"{reference}\"")),
        _ => Err(format!("\"{reference}\" matches more than one profile")),
    }
}

async fn profile_id(client: &Client, reference: &str) -> Result<String, String> {
    let profiles = client.list_profiles().await.map_err(fail)?;
    find_profile(&profiles, reference).map(|profile| profile.id.clone())
}

fn describe(state: &TunnelState) -> String {
    match state {
        TunnelState::Disconnected => "disconnected".into(),
        TunnelState::Connecting { step } => format!("connecting ({step:?})").to_lowercase(),
        TunnelState::Connected { .. } => "connected".into(),
        TunnelState::Disconnecting => "disconnecting".into(),
        TunnelState::Failed { reason } => format!("failed: {reason}"),
    }
}

fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

fn print_stats(stats: &Stats) {
    println!(
        "down {}/s  up {}/s  total {} down, {} up  flows {}  latency {}",
        bytes(stats.download_bps),
        bytes(stats.upload_bps),
        bytes(stats.received_bytes),
        bytes(stats.sent_bytes),
        stats.active_flows,
        stats
            .latency_ms
            .map_or("-".to_owned(), |ms| format!("{ms} ms")),
    );
}

fn print_status(status: &Status, profiles: &[Profile], json: bool) {
    if json {
        println!("{}", serde_json::to_string(status).unwrap_or_default());
        return;
    }
    println!("{}", describe(&status.state));
    if let Some(profile) = status
        .profile_id
        .as_deref()
        .and_then(|id| profiles.iter().find(|profile| profile.id == id))
    {
        println!("profile: {} ({})", profile.name, profile.server_list());
    }
    if let Some(stats) = &status.stats {
        print_stats(stats);
    }
    if status.restart_required {
        println!("settings changed: reconnect to apply them");
    }
}

fn print_log(entry: &LogEntry) {
    let level = match entry.level {
        LogLevel::Trace => "TRACE",
        LogLevel::Debug => "DEBUG",
        LogLevel::Info => "INFO",
        LogLevel::Warn => "WARN",
        LogLevel::Error => "ERROR",
    };
    println!("{} {level:<5} {}", entry.timestamp_ms, entry.message);
}

async fn run(cli: Cli) -> Result<(), String> {
    let socket = cli.socket.unwrap_or_else(fiend_ipc::path::default_socket);
    let client = Client::connect(&socket, "fiend-cli").await.map_err(fail)?;
    let json = cli.json;
    match cli.command {
        Command::Status => {
            let status = client.status().await.map_err(fail)?;
            let profiles = client.list_profiles().await.map_err(fail)?;
            print_status(&status, &profiles, json);
        }
        Command::Connect { profile, no_wait } => {
            let id = match profile {
                Some(reference) => Some(profile_id(&client, &reference).await?),
                None => None,
            };
            client.subscribe().await.map_err(fail)?;
            let mut events = client.events();
            client.connect_profile(id).await.map_err(fail)?;
            if no_wait {
                return Ok(());
            }
            tokio::time::timeout(Duration::from_secs(60), async {
                let status = client.status().await.map_err(fail)?;
                if let Some(result) = settled(&status.state) {
                    return result;
                }
                loop {
                    match events.recv().await {
                        Ok(Event::State(status)) => {
                            if let Some(result) = settled(&status.state) {
                                return result;
                            }
                        }
                        Ok(_) => {}
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(_) => return Err("the daemon closed the connection".to_owned()),
                    }
                }
            })
            .await
            .map_err(|_| "timeout while waiting for the connection".to_owned())??;
            let status = client.status().await.map_err(fail)?;
            let profiles = client.list_profiles().await.map_err(fail)?;
            print_status(&status, &profiles, json);
        }
        Command::Disconnect => {
            client.disconnect().await.map_err(fail)?;
            let status = client.status().await.map_err(fail)?;
            print_status(&status, &[], json);
        }
        Command::Watch => {
            client.subscribe().await.map_err(fail)?;
            let mut events = client.events();
            let status = client.status().await.map_err(fail)?;
            print_status(&status, &[], json);
            loop {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => break,
                    event = events.recv() => match event {
                        Ok(Event::State(status)) => print_status(&status, &[], json),
                        Ok(Event::Stats(stats)) if json => {
                            println!("{}", serde_json::to_string(&stats).unwrap_or_default());
                        }
                        Ok(Event::Stats(stats)) => print_stats(&stats),
                        Ok(Event::Log(entry)) => print_log(&entry),
                        Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                        Err(_) => return Err("the daemon closed the connection".into()),
                    }
                }
            }
        }
        Command::Profiles(command) => profiles(&client, command, json).await?,
        Command::Logs {
            lines,
            follow,
            clear,
        } => {
            if clear {
                return client.clear_logs().await.map_err(fail);
            }
            if follow {
                client.subscribe().await.map_err(fail)?;
            }
            let mut events = client.events();
            for entry in client.logs(lines).await.map_err(fail)? {
                print_log(&entry);
            }
            if follow {
                loop {
                    tokio::select! {
                        _ = tokio::signal::ctrl_c() => break,
                        event = events.recv() => match event {
                            Ok(Event::Log(entry)) => print_log(&entry),
                            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                            Err(_) => return Err("the daemon closed the connection".into()),
                        }
                    }
                }
            }
        }
        Command::Settings => {
            let settings: Settings = client.settings().await.map_err(fail)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&settings).unwrap_or_default()
            );
        }
        Command::Rules { kind, file } => {
            let text = read_input(&file).await?;
            let stats = client
                .set_rule_text(kind.into(), text)
                .await
                .map_err(fail)?;
            println!(
                "{} domains, {} exact, {} keywords, {} networks",
                stats.domains, stats.exact, stats.keywords, stats.cidrs
            );
        }
        Command::Subscriptions(command) => subscriptions(&client, command).await?,
    }
    Ok(())
}

fn settled(state: &TunnelState) -> Option<Result<(), String>> {
    match state {
        TunnelState::Connected { .. } => Some(Ok(())),
        TunnelState::Failed { reason } => Some(Err(reason.clone())),
        TunnelState::Disconnected => Some(Err("the connection stopped".to_owned())),
        _ => None,
    }
}

async fn profiles(client: &Client, command: ProfileCommand, json: bool) -> Result<(), String> {
    match command {
        ProfileCommand::List => {
            let profiles = client.list_profiles().await.map_err(fail)?;
            let active = client.settings().await.map_err(fail)?.active_profile;
            for profile in &profiles {
                if json {
                    let value = serde_json::json!({
                        "id": profile.id, "name": profile.name, "servers": profile.servers,
                        "active": active.as_deref() == Some(profile.id.as_str()),
                    });
                    println!("{value}");
                } else {
                    let mark = if active.as_deref() == Some(profile.id.as_str()) {
                        '*'
                    } else {
                        ' '
                    };
                    println!(
                        "{mark} {}  {}  {}",
                        &profile.id[..8],
                        profile.name,
                        profile.server_list()
                    );
                }
            }
        }
        ProfileCommand::Import { link } => {
            let link = if link == "-" {
                read_input("-").await?.trim().to_owned()
            } else {
                link
            };
            let profile = client.import_link(link).await.map_err(fail)?;
            println!("imported {} ({})", profile.name, &profile.id[..8]);
        }
        ProfileCommand::Share { profile } => {
            let id = profile_id(client, &profile).await?;
            println!("{}", client.share_link(id).await.map_err(fail)?);
        }
        ProfileCommand::Delete { profile } => {
            let id = profile_id(client, &profile).await?;
            client.delete_profile(id).await.map_err(fail)?;
        }
        ProfileCommand::Latency { profile } => {
            let id = profile_id(client, &profile).await?;
            println!("{} ms", client.measure_latency(id).await.map_err(fail)?);
        }
    }
    Ok(())
}

async fn subscriptions(client: &Client, command: SubscriptionCommand) -> Result<(), String> {
    match command {
        SubscriptionCommand::List => {
            for item in client.settings().await.map_err(fail)?.subscriptions {
                println!(
                    "{}  {:?}  {} entries  {}{}",
                    &item.id[..8],
                    item.kind,
                    item.entries,
                    item.url,
                    item.error
                        .map_or(String::new(), |e| format!("  error: {e}")),
                );
            }
        }
        SubscriptionCommand::Add { url, kind } => {
            let item = client
                .add_subscription(url, kind.into())
                .await
                .map_err(fail)?;
            match item.error {
                Some(error) => return Err(format!("added, but the download failed: {error}")),
                None => println!("added {} ({} entries)", &item.id[..8], item.entries),
            }
        }
        SubscriptionCommand::Remove { id } => {
            let settings = client.settings().await.map_err(fail)?;
            let matches: Vec<_> = settings
                .subscriptions
                .iter()
                .filter(|item| item.id.starts_with(&id))
                .collect();
            match matches.as_slice() {
                [one] => client
                    .remove_subscription(one.id.clone())
                    .await
                    .map_err(fail)?,
                [] => return Err(format!("no subscription matches \"{id}\"")),
                _ => return Err(format!("\"{id}\" matches more than one subscription")),
            }
        }
        SubscriptionCommand::Refresh => {
            client.refresh_subscriptions().await.map_err(fail)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(id: &str, name: &str) -> Profile {
        Profile {
            id: id.into(),
            name: name.into(),
            servers: Vec::new(),
            secret: String::new(),
            template_json: String::new(),
            tuning: Default::default(),
        }
    }

    #[test]
    fn profiles_are_found_by_id_prefix_or_name() {
        let list = [profile("abc123", "Home"), profile("abd456", "Work")];
        assert_eq!(find_profile(&list, "Home").unwrap().id, "abc123");
        assert_eq!(find_profile(&list, "abc").unwrap().id, "abc123");
        assert_eq!(find_profile(&list, "abd456").unwrap().id, "abd456");
        assert!(find_profile(&list, "ab").is_err());
        assert!(find_profile(&list, "zzz").is_err());
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(bytes(10), "10 B");
        assert_eq!(bytes(1536), "1.5 KiB");
        assert_eq!(bytes(5 << 30), "5.0 GiB");
    }

    #[test]
    fn connect_waits_for_a_final_state() {
        assert!(settled(&TunnelState::Disconnecting).is_none());
        assert!(
            settled(&TunnelState::Connected { since_ms: 1 })
                .unwrap()
                .is_ok()
        );
        assert!(
            settled(&TunnelState::Failed { reason: "x".into() })
                .unwrap()
                .is_err()
        );
    }
}
