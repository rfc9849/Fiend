#[cfg(any(unix, windows))]
#[path = "e2e/unix.rs"]
mod e2e_desktop;

#[cfg(any(unix, windows))]
fn main() {
    e2e_desktop::main();
}

#[cfg(not(any(unix, windows)))]
fn main() {}
