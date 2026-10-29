#![no_main]

use fiend_ipc::{ClientMessage, HANDSHAKE_FRAME_BYTES, MAX_FRAME_BYTES, ServerMessage, decode_frame};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = decode_frame::<ClientMessage>(data, HANDSHAKE_FRAME_BYTES);
    let _ = decode_frame::<ClientMessage>(data, MAX_FRAME_BYTES);
    let _ = decode_frame::<ServerMessage>(data, MAX_FRAME_BYTES);
});
