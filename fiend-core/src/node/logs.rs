use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};

const MAX_LINES: usize = 256;
const MAX_LINE_BYTES: usize = 2048;
static LINES: OnceLock<Mutex<VecDeque<String>>> = OnceLock::new();

#[derive(Default)]
pub struct Writer {
    pending: Vec<u8>,
}

impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        io::stderr().lock().write_all(bytes)?;
        for byte in bytes {
            if *byte == b'\n' {
                self.push_line();
            } else if self.pending.len() < MAX_LINE_BYTES {
                self.pending.push(*byte);
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        io::stderr().lock().flush()
    }
}

impl Writer {
    fn push_line(&mut self) {
        let line = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        let mut lines = LINES
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if lines.len() >= MAX_LINES {
            lines.pop_front();
        }
        lines.push_back(line);
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        if !self.pending.is_empty() {
            self.push_line();
        }
    }
}

pub fn recent(limit: usize) -> Vec<String> {
    let lines = LINES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    lines
        .iter()
        .skip(lines.len().saturating_sub(limit.min(MAX_LINES)))
        .cloned()
        .collect()
}
