#[cfg(any(target_os = "linux", target_os = "android"))]
mod helper {
    use std::io;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
    use std::os::fd::{AsRawFd, RawFd};
    use std::os::unix::net::UnixStream;
    use std::path::Path;
    use std::process::{Command, ExitCode, Stdio};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use fiend_root::frame;
    use fiend_root::protocol::{Policy, Request, Response};
    use fiend_root::rules;
    use nix::fcntl::{Flock, FlockArg};
    use nix::sys::signal::{
        SaFlags, SigAction, SigHandler, SigSet, SigmaskHow, Signal, pthread_sigmask, sigaction,
    };
    use nix::sys::socket::{getsockopt, setsockopt, sockopt};
    use socket2::{Domain, Protocol, Socket, Type};

    static STOP: AtomicBool = AtomicBool::new(false);
    const SIGNALS: [Signal; 3] = [Signal::SIGTERM, Signal::SIGINT, Signal::SIGHUP];
    const CHECK_PERIOD: Duration = Duration::from_secs(10);
    const STEP: Duration = Duration::from_millis(250);
    const BACKLOG: i32 = 1024;
    const PORT_ATTEMPTS: usize = 32;
    const LOCK: &str = "/data/adb/fiend-root.lock";
    const LOCK_WAIT: Duration = Duration::from_secs(15);

    extern "C" fn on_signal(_: nix::libc::c_int) {
        STOP.store(true, Ordering::SeqCst);
    }

    struct Active {
        policy: Policy,
        port: u16,
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
        for command in rules::apply(&active.policy, active.port) {
            run(&command)?;
        }
        Ok(())
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
                            },
                            &fds,
                        )?;
                    }
                    Err(error) => frame::send(stream, &failure(error), &[])?,
                },
                Request::Reply { address } => {
                    let running = state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .is_some();
                    let socket = if running {
                        reply(address)
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
            let port = random_port();
            match sockets(port, policy.ipv6) {
                Ok((tcp, udp)) => {
                    let next = Active { policy, port };
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
                cleanup();
                if let Err(error) = apply(active) {
                    eprintln!("cannot add the rules again: {error}");
                }
            }
        }
    }

    fn random_port() -> u16 {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|time| time.as_nanos() as u64)
            .unwrap_or(0);
        let mut seed = nanos
            ^ (u64::from(std::process::id()) << 32)
            ^ COUNTER
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_mul(0x9e37_79b9_7f4a_7c15);
        seed ^= seed >> 33;
        seed = seed.wrapping_mul(0xff51_afd7_ed55_8ccd);
        seed ^= seed >> 33;
        20000 + (seed % 40000) as u16
    }

    fn transparent(socket: &Socket, ipv6: bool) -> io::Result<()> {
        socket.set_reuse_address(true)?;
        if ipv6 {
            socket.set_only_v6(true)?;
            socket.set_ip_transparent_v6(true)
        } else {
            socket.set_ip_transparent_v4(true)
        }
    }

    fn sockets(port: u16, ipv6: bool) -> io::Result<(Vec<Socket>, Vec<Socket>)> {
        let mut loopbacks = vec![IpAddr::V4(Ipv4Addr::LOCALHOST)];
        if ipv6 {
            loopbacks.push(IpAddr::V6(Ipv6Addr::LOCALHOST));
        }
        let mut tcp = Vec::new();
        let mut udp = Vec::new();
        for ip in loopbacks {
            let address = SocketAddr::new(ip, port);
            let v6 = ip.is_ipv6();
            let listener = Socket::new(
                Domain::for_address(address),
                Type::STREAM,
                Some(Protocol::TCP),
            )?;
            transparent(&listener, v6)?;
            listener.bind(&address.into())?;
            listener.listen(BACKLOG)?;
            tcp.push(listener);
            let receiver = Socket::new(
                Domain::for_address(address),
                Type::DGRAM,
                Some(Protocol::UDP),
            )?;
            transparent(&receiver, v6)?;
            if v6 {
                setsockopt(&receiver, sockopt::Ipv6OrigDstAddr, &true)?;
            } else {
                setsockopt(&receiver, sockopt::Ipv4OrigDstAddr, &true)?;
            }
            receiver.bind(&address.into())?;
            udp.push(receiver);
        }
        Ok((tcp, udp))
    }

    fn reply(address: SocketAddr) -> io::Result<Socket> {
        if address.ip().is_loopback() || address.ip().is_unspecified() || address.port() == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reply address is not valid",
            ));
        }
        let socket = Socket::new(
            Domain::for_address(address),
            Type::DGRAM,
            Some(Protocol::UDP),
        )?;
        transparent(&socket, address.is_ipv6())?;
        socket.bind(&address.into())?;
        Ok(socket)
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
