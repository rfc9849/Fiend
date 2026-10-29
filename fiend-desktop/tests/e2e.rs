#[cfg(unix)]
#[path = "e2e/unix.rs"]
mod e2e_unix;

#[cfg(unix)]
fn main() {
    e2e_unix::main();
}

#[cfg(not(unix))]
fn main() {}
