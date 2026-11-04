#[cfg(any(target_os = "linux", target_os = "android"))]
mod helper {
    use std::io;
    use std::os::fd::{AsRawFd, RawFd};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::process::{Command, ExitCode, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use fiend_root::bpf;
    use fiend_root::frame;
    use fiend_root::protocol::{PROTOCOL, Policy, Request, Response};
    use fiend_root::rules;
    use nix::fcntl::{Flock, FlockArg};
    use nix::sys::signal::{
        SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal, pthread_sigmask, sigaction,
    };
    use nix::sys::socket::{getsockopt, sockopt};
    use socket2::Socket;

    static STOP: AtomicBool = AtomicBool::new(false);
    const SIGNALS: [Signal; 3] = [Signal::SIGTERM, Signal::SIGINT, Signal::SIGHUP];
    const CHECK_PERIOD: Duration = Duration::from_secs(10);
    const STEP: Duration = Duration::from_millis(250);
    const PORT_ATTEMPTS: usize = 32;
    const LOCK: &str = "/data/adb/fiend-root.lock";
    const LOCK_WAIT: Duration = Duration::from_secs(15);

    extern "C" fn on_signal(_: nix::libc::c_int) {
        STOP.store(true, Ordering::SeqCst);
    }

    struct Active {
        policy: Policy,
        port: u16,
        support: rules::Support,
        data_saver: Vec<(rules::Family, bool)>,
    }

    pub fn main() -> ExitCode {
        let args: Vec<String> = std::env::args().collect();
        let result = match (args.get(1).map(String::as_str), args.len()) {
            (Some("attach"), 3) => attach(Path::new(&args[2])),
            (Some("cleanup"), 2) => lock().map(|_lock| cleanup()),
            (Some("version"), 2) => {
                println!("{}", env!("CARGO_PKG_VERSION"));
                Ok(())
            }
            _ => {
                eprintln!("usage: fiend-root attach <socket> | cleanup | version");
                return ExitCode::from(2);
            }
        };
        match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("fiend-root: {error}");
                ExitCode::FAILURE
            }
        }
    }

    fn run(command: &[String]) -> io::Result<()> {
        let output = Command::new(&command[0])
            .args(&command[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()?;
        if output.status.success() {
            return Ok(());
        }
        Err(io::Error::other(format!(
            "{}: {}",
            command.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }

    fn cleanup() {
        remove_rules();
        let _ = std::fs::remove_file(bpf::BACKGROUND_PROGRAM);
        let _ = std::fs::remove_file(bpf::UID_PROGRAM);
    }

    fn remove_rules() {
        for (repeat, command) in rules::cleanup() {
            let times = if repeat { rules::CLEANUP_REPEATS } else { 1 };
            for _ in 0..times {
                if run(&command).is_err() {
                    break;
                }
            }
        }
    }

    fn apply(active: &Active) -> io::Result<()> {
        for command in rules::apply(&active.policy, active.port, &active.support) {
            run(&command)?;
        }
        follow_metered(active)
    }

    fn follow_metered(active: &Active) -> io::Result<()> {
        for (family, present) in &active.data_saver {
            for command in rules::metered(*family, active.policy.metered, *present) {
                run(&command)?;
            }
        }
        Ok(())
    }

    /// Loads the program for the background rule of Android. Without it the
    /// captured apps can use the network in the background.
    fn support(policy: &Policy) -> rules::Support {
        let background = bpf::pin_background_program(
            Path::new(bpf::CONFIGURATION_MAP),
            Path::new(bpf::UID_OWNER_MAP),
            Path::new(bpf::BACKGROUND_PROGRAM),
        )
        .map(|()| bpf::BACKGROUND_PROGRAM.to_owned())
        .inspect_err(|error| eprintln!("the Android background rule is not followed: {error}"))
        .ok();
        let uids = if policy.dns {
            captured_uids(policy)
                .and_then(|uids| bpf::pin_uid_program(&uids, Path::new(bpf::UID_PROGRAM)))
                .map(|()| bpf::UID_PROGRAM.to_owned())
                .inspect_err(|error| {
                    eprintln!("resolver queries are matched by file owner: {error}")
                })
                .ok()
        } else {
            None
        };
        rules::Support { background, uids }
    }

    fn captured_uids(policy: &Policy) -> io::Result<Vec<u32>> {
        let mut uids = Vec::new();
        for range in &policy.include {
            for uid in range.first..=range.last {
                if !policy.exclude.contains(&uid) {
                    uids.push(uid);
                }
                if uids.len() > bpf::MAX_UIDS {
                    return Err(io::Error::other("too many UIDs"));
                }
            }
        }
        uids.sort_unstable();
        uids.dedup();
        Ok(uids)
    }

    fn data_saver(policy: &Policy) -> Vec<(rules::Family, bool)> {
        rules::families(policy)
            .into_iter()
            .map(|family| {
                let command: Vec<String> = [
                    family.iptables(),
                    "-w",
                    "-t",
                    "filter",
                    "-n",
                    "-L",
                    rules::DATA_SAVER_CHAIN,
                ]
                .iter()
                .map(|part| (*part).to_owned())
                .collect();
                let present = run(&command).is_ok();
                if !present {
                    eprintln!("no Data Saver chain for {family:?}");
                }
                (family, present)
            })
            .collect()
    }

    fn intact(policy: &Policy) -> bool {
        rules::check(policy)
            .iter()
            .all(|command| run(command).is_ok())
    }

    fn signals(how: SigmaskHow) -> io::Result<()> {
        let mut set = SigSet::empty();
        for signal in SIGNALS {
            set.add(signal);
        }
        pthread_sigmask(how, Some(&set), None)?;
        Ok(())
    }

    /// One helper owns the rules at a time. A helper that is still removing
    /// its rules keeps the lock until it exits, so a new helper waits.
    fn lock() -> io::Result<Flock<std::fs::File>> {
        let path = std::env::var_os("FIEND_ROOT_LOCK").unwrap_or_else(|| LOCK.into());
        let deadline = std::time::Instant::now() + LOCK_WAIT;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path)?;
        loop {
            match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
                Ok(lock) => return Ok(lock),
                Err((back, nix::errno::Errno::EWOULDBLOCK)) => file = back,
                Err((_, error)) => return Err(error.into()),
            }
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::other("Another root helper does not stop"));
            }
            std::thread::sleep(STEP);
        }
    }

    fn attach(path: &Path) -> io::Result<()> {
        let stream = UnixStream::connect(path)?;
        let _lock = lock()?;
        let peer = getsockopt(&stream, sockopt::PeerCredentials)?;
        let label = std::fs::read_to_string(format!("/proc/{}/attr/current", peer.pid()))
            .ok()
            .map(|label| label.trim_end_matches(['\0', '\n']).to_owned())
            .filter(|label| !label.is_empty());
        let action = SigAction::new(
            SigHandler::Handler(on_signal),
            SaFlags::empty(),
            SigSet::empty(),
        );
        for signal in SIGNALS {
            // SAFETY: the handler only stores to an atomic flag.
            unsafe { sigaction(signal, &action) }?;
        }
        let state: Arc<Mutex<Option<Active>>> = Arc::new(Mutex::new(None));
        let done = Arc::new(AtomicBool::new(false));
        signals(SigmaskHow::SIG_BLOCK)?;
        let watchdog = {
            let state = state.clone();
            let done = done.clone();
            std::thread::Builder::new()
                .name("fiend-root-check".into())
                .spawn(move || watch(&state, &done))?
        };
        signals(SigmaskHow::SIG_UNBLOCK)?;
        let result = serve(&stream, label.as_deref(), &state);
        done.store(true, Ordering::SeqCst);
        let _ = watchdog.join();
        let active = state.lock().unwrap_or_else(|error| error.into_inner());
        if active.is_some() {
            cleanup();
            eprintln!("rules removed");
        }
        result
    }

    fn serve(
        stream: &UnixStream,
        label: Option<&str>,
        state: &Mutex<Option<Active>>,
    ) -> io::Result<()> {
        loop {
            if STOP.load(Ordering::SeqCst) {
                return Ok(());
            }
            let request = match frame::receive::<Request>(stream) {
                Ok(Some((request, _))) => request,
                Ok(None) => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                    frame::send(stream, &failure(error), &[])?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            match request {
                Request::Start { policy } => match start(policy, label, state) {
                    Ok((tcp, udp)) => {
                        let fds: Vec<RawFd> =
                            tcp.iter().chain(&udp).map(AsRawFd::as_raw_fd).collect();
                        frame::send(
                            stream,
                            &Response::Ready {
                                tcp: tcp.len(),
                                udp: udp.len(),
                                protocol: PROTOCOL,
                            },
                            &fds,
                        )?;
                    }
                    Err(error) => frame::send(stream, &failure(error), &[])?,
                },
                Request::Metered { metered } => {
                    let mut active = state.lock().unwrap_or_else(|error| error.into_inner());
                    let result = match active.as_mut() {
                        Some(active) => {
                            active.policy.metered = metered;
                            follow_metered(active)
                        }
                        None => Err(io::Error::other("The rules are not in place")),
                    };
                    drop(active);
                    match result {
                        Ok(()) => frame::send(stream, &Response::Done, &[])?,
                        Err(error) => frame::send(stream, &failure(error), &[])?,
                    }
                }
                Request::Reply { address } => {
                    let running = state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .is_some();
                    let socket = if running {
                        fiend_root::sockets::reply(address, None)
                    } else {
                        Err(io::Error::other("The rules are not in place"))
                    };
                    match socket {
                        Ok(socket) => {
                            frame::send(stream, &Response::Socket, &[socket.as_raw_fd()])?
                        }
                        Err(error) => frame::send(stream, &failure(error), &[])?,
                    }
                }
            }
        }
    }

    fn failure(error: io::Error) -> Response {
        Response::Failed {
            message: error.to_string(),
        }
    }

    fn start(
        policy: Policy,
        label: Option<&str>,
        state: &Mutex<Option<Active>>,
    ) -> io::Result<(Vec<Socket>, Vec<Socket>)> {
        let mut active = state.lock().unwrap_or_else(|error| error.into_inner());
        if active.is_some() {
            return Err(io::Error::other("The rules are already in place"));
        }
        policy.validate().map_err(io::Error::other)?;
        if let Some(label) = label
            && let Err(error) = std::fs::write("/proc/thread-self/attr/sockcreate", label)
        {
            eprintln!("cannot label the sockets: {error}");
        }
        cleanup();
        let mut last = io::Error::other("no free port");
        for _ in 0..PORT_ATTEMPTS {
            let port = fiend_root::sockets::random_port();
            match fiend_root::sockets::listeners(port, policy.ipv6, None) {
                Ok((tcp, udp)) => {
                    let data_saver = data_saver(&policy);
                    let support = support(&policy);
                    let next = Active {
                        policy,
                        port,
                        support,
                        data_saver,
                    };
                    if let Err(error) = apply(&next) {
                        cleanup();
                        return Err(error);
                    }
                    eprintln!("rules added for port {port}");
                    *active = Some(next);
                    return Ok((tcp, udp));
                }
                Err(error) if error.kind() == io::ErrorKind::AddrInUse => last = error,
                Err(error) => return Err(error),
            }
        }
        Err(last)
    }

    fn watch(state: &Mutex<Option<Active>>, done: &AtomicBool) {
        loop {
            let mut waited = Duration::ZERO;
            while waited < CHECK_PERIOD {
                if done.load(Ordering::SeqCst) || STOP.load(Ordering::SeqCst) {
                    return;
                }
                std::thread::sleep(STEP);
                waited += STEP;
            }
            let active = state.lock().unwrap_or_else(|error| error.into_inner());
            if let Some(active) = active.as_ref()
                && !intact(&active.policy)
            {
                eprintln!("rules are missing, adding them again");
                remove_rules();
                if let Err(error) = apply(active) {
                    eprintln!("cannot add the rules again: {error}");
                }
            }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn main() -> std::process::ExitCode {
    helper::main()
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn main() -> std::process::ExitCode {
    eprintln!("fiend-root runs only on Linux and Android");
    std::process::ExitCode::FAILURE
}
