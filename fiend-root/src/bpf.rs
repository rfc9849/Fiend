//! Socket filters for `iptables -m bpf`.
//!
//! The background program matches packets that Android blocks with its
//! background rule. netd skips this rule for loopback, and the captured
//! packets go through loopback, so the helper checks it here. The program
//! reads the same netd maps as the netd egress program.
//!
//! The UID program matches packets of sockets that belong to a captured app.
//! The Android resolver runs in netd and gives each query socket to the app
//! with `fchown`, which changes the socket UID but not the file owner that
//! `-m owner` checks.

use std::ffi::CString;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::Path;

pub const CONFIGURATION_MAP: &str = "/sys/fs/bpf/netd_shared/map_netd_configuration_map";
pub const UID_OWNER_MAP: &str = "/sys/fs/bpf/netd_shared/map_netd_uid_owner_map";
pub const BACKGROUND_PROGRAM: &str = "/sys/fs/bpf/fiend_background";
pub const UID_PROGRAM: &str = "/sys/fs/bpf/fiend_uids";
pub const MAX_UIDS: usize = 32_768;

const BACKGROUND_MATCH: i32 = 1 << 12;
const UID_RULES_CONFIGURATION_KEY: i32 = 0;
const AID_USER_OFFSET: i32 = 100_000;
const AID_APP_START: i32 = 10_000;
const OVERFLOW_UID: i32 = 65_534;

const BPF_MAP_LOOKUP_ELEM: i32 = 1;
const BPF_MAP_CREATE: u32 = 0;
const BPF_MAP_UPDATE_ELEM: u32 = 2;
const BPF_MAP_TYPE_HASH: u32 = 1;
const BPF_GET_SOCKET_UID: i32 = 47;
const BPF_PROG_LOAD: u32 = 5;
const BPF_OBJ_PIN: u32 = 6;
const BPF_OBJ_GET: u32 = 7;
const BPF_PROG_TYPE_SOCKET_FILTER: u32 = 1;
const BPF_F_RDONLY: u32 = 1 << 3;
const BPF_PSEUDO_MAP_FD: u8 = 1;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Instruction {
    code: u8,
    registers: u8,
    offset: i16,
    immediate: i32,
}

fn instruction(code: u8, destination: u8, source: u8, offset: i16, immediate: i32) -> Instruction {
    Instruction {
        code,
        registers: (source << 4) | destination,
        offset,
        immediate,
    }
}

const MOV64_REGISTER: u8 = 0xbf;
const MOV64_IMMEDIATE: u8 = 0xb7;
const ADD64_IMMEDIATE: u8 = 0x07;
const AND64_IMMEDIATE: u8 = 0x57;
const MOD64_IMMEDIATE: u8 = 0x97;
const STORE_WORD_REGISTER: u8 = 0x63;
const STORE_WORD_IMMEDIATE: u8 = 0x62;
const LOAD_WORD: u8 = 0x61;
const LOAD_IMMEDIATE64: u8 = 0x18;
const JUMP_EQUAL: u8 = 0x15;
const JUMP_NOT_EQUAL: u8 = 0x55;
const JUMP_LESS: u8 = 0xa5;
const CALL: u8 = 0x85;
const EXIT: u8 = 0x95;

/// Returns 1 for an app socket when the background rule is on and the UID
/// does not have the background allow bit, the same test as
/// `isBlockedByUidRules` in netd for this one rule.
fn background_program(configuration: i32, owners: i32) -> Vec<Instruction> {
    const MATCH: i16 = 25;
    const NO_MATCH: i16 = 27;
    let jump = |at: i16, target: i16| target - at - 1;
    vec![
        instruction(CALL, 0, 0, 0, BPF_GET_SOCKET_UID),
        instruction(STORE_WORD_REGISTER, 10, 0, -4, 0),
        instruction(JUMP_EQUAL, 0, 0, jump(2, NO_MATCH), OVERFLOW_UID),
        instruction(MOV64_REGISTER, 1, 0, 0, 0),
        instruction(MOD64_IMMEDIATE, 1, 0, 0, AID_USER_OFFSET),
        instruction(JUMP_LESS, 1, 0, jump(5, NO_MATCH), AID_APP_START),
        instruction(STORE_WORD_IMMEDIATE, 10, 0, -8, UID_RULES_CONFIGURATION_KEY),
        instruction(LOAD_IMMEDIATE64, 1, BPF_PSEUDO_MAP_FD, 0, configuration),
        Instruction::default(),
        instruction(MOV64_REGISTER, 2, 10, 0, 0),
        instruction(ADD64_IMMEDIATE, 2, 0, 0, -8),
        instruction(CALL, 0, 0, 0, BPF_MAP_LOOKUP_ELEM),
        instruction(JUMP_EQUAL, 0, 0, jump(12, NO_MATCH), 0),
        instruction(LOAD_WORD, 7, 0, 0, 0),
        instruction(AND64_IMMEDIATE, 7, 0, 0, BACKGROUND_MATCH),
        instruction(JUMP_EQUAL, 7, 0, jump(15, NO_MATCH), 0),
        instruction(LOAD_IMMEDIATE64, 1, BPF_PSEUDO_MAP_FD, 0, owners),
        Instruction::default(),
        instruction(MOV64_REGISTER, 2, 10, 0, 0),
        instruction(ADD64_IMMEDIATE, 2, 0, 0, -4),
        instruction(CALL, 0, 0, 0, BPF_MAP_LOOKUP_ELEM),
        instruction(JUMP_EQUAL, 0, 0, jump(21, MATCH), 0),
        instruction(LOAD_WORD, 1, 0, 4, 0),
        instruction(AND64_IMMEDIATE, 1, 0, 0, BACKGROUND_MATCH),
        instruction(JUMP_NOT_EQUAL, 1, 0, jump(24, NO_MATCH), 0),
        instruction(MOV64_IMMEDIATE, 0, 0, 0, 1),
        instruction(EXIT, 0, 0, 0, 0),
        instruction(MOV64_IMMEDIATE, 0, 0, 0, 0),
        instruction(EXIT, 0, 0, 0, 0),
    ]
}

/// Returns 1 when the socket UID is a key of the map.
fn uid_program(uids: i32) -> Vec<Instruction> {
    vec![
        instruction(CALL, 0, 0, 0, BPF_GET_SOCKET_UID),
        instruction(STORE_WORD_REGISTER, 10, 0, -4, 0),
        instruction(LOAD_IMMEDIATE64, 1, BPF_PSEUDO_MAP_FD, 0, uids),
        Instruction::default(),
        instruction(MOV64_REGISTER, 2, 10, 0, 0),
        instruction(ADD64_IMMEDIATE, 2, 0, 0, -4),
        instruction(CALL, 0, 0, 0, BPF_MAP_LOOKUP_ELEM),
        instruction(JUMP_EQUAL, 0, 0, 2, 0),
        instruction(MOV64_IMMEDIATE, 0, 0, 0, 1),
        instruction(EXIT, 0, 0, 0, 0),
        instruction(MOV64_IMMEDIATE, 0, 0, 0, 0),
        instruction(EXIT, 0, 0, 0, 0),
    ]
}

#[repr(C)]
#[derive(Default)]
struct MapAttributes {
    map_type: u32,
    key_size: u32,
    value_size: u32,
    max_entries: u32,
    map_flags: u32,
}

#[repr(C)]
#[derive(Default)]
struct ElementAttributes {
    map_fd: u32,
    padding: u32,
    key: u64,
    value: u64,
    flags: u64,
}

#[repr(C)]
#[derive(Default)]
struct ObjectAttributes {
    pathname: u64,
    bpf_fd: u32,
    file_flags: u32,
}

#[repr(C)]
#[derive(Default)]
struct LoadAttributes {
    prog_type: u32,
    insn_cnt: u32,
    insns: u64,
    license: u64,
    log_level: u32,
    log_size: u32,
    log_buf: u64,
    kern_version: u32,
    prog_flags: u32,
}

fn bpf<T>(command: u32, attributes: &mut T) -> io::Result<i64> {
    // SAFETY: `attributes` is a live, initialised `repr(C)` value of the
    // layout that the kernel expects for `command`, and its size is passed.
    let result = unsafe {
        nix::libc::syscall(
            nix::libc::SYS_bpf,
            command,
            attributes as *mut T,
            std::mem::size_of::<T>(),
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(result)
}

fn owned(result: i64) -> io::Result<OwnedFd> {
    let fd = i32::try_from(result).map_err(io::Error::other)?;
    // SAFETY: the kernel returned a new descriptor that nothing else owns.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn path(value: &Path) -> io::Result<CString> {
    CString::new(value.as_os_str().as_encoded_bytes()).map_err(io::Error::other)
}

fn object(value: &Path) -> io::Result<OwnedFd> {
    let name = path(value)?;
    let mut attributes = ObjectAttributes {
        pathname: name.as_ptr() as u64,
        file_flags: BPF_F_RDONLY,
        ..ObjectAttributes::default()
    };
    owned(bpf(BPF_OBJ_GET, &mut attributes)?)
}

/// Loads the program and pins it at `pin`. Fails when the netd maps are
/// absent, for example on desktop Linux.
pub fn pin_background_program(configuration: &Path, owners: &Path, pin: &Path) -> io::Result<()> {
    let configuration = object(configuration)?;
    let owners = object(owners)?;
    load_and_pin(
        &background_program(configuration.as_raw_fd(), owners.as_raw_fd()),
        pin,
    )
}

/// Makes a map of `uids`, loads the UID program for it and pins the program
/// at `pin`. The program keeps the map alive.
pub fn pin_uid_program(uids: &[u32], pin: &Path) -> io::Result<()> {
    if uids.is_empty() || uids.len() > MAX_UIDS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the number of UIDs is not valid",
        ));
    }
    let mut attributes = MapAttributes {
        map_type: BPF_MAP_TYPE_HASH,
        key_size: 4,
        value_size: 4,
        max_entries: uids.len() as u32,
        map_flags: 0,
    };
    let map = owned(bpf(BPF_MAP_CREATE, &mut attributes)?)?;
    let value = 1u32;
    for uid in uids {
        let mut attributes = ElementAttributes {
            map_fd: map.as_raw_fd() as u32,
            key: uid as *const u32 as u64,
            value: &value as *const u32 as u64,
            ..ElementAttributes::default()
        };
        bpf(BPF_MAP_UPDATE_ELEM, &mut attributes)?;
    }
    load_and_pin(&uid_program(map.as_raw_fd()), pin)
}

fn load_and_pin(program: &[Instruction], pin: &Path) -> io::Result<()> {
    let license = CString::new("PolyForm-Noncommercial-1.0.0").map_err(io::Error::other)?;
    let mut log = vec![0u8; 16 << 10];
    let mut attributes = LoadAttributes {
        prog_type: BPF_PROG_TYPE_SOCKET_FILTER,
        insn_cnt: program.len() as u32,
        insns: program.as_ptr() as u64,
        license: license.as_ptr() as u64,
        log_level: 1,
        log_size: log.len() as u32,
        log_buf: log.as_mut_ptr() as u64,
        ..LoadAttributes::default()
    };
    let loaded = bpf(BPF_PROG_LOAD, &mut attributes).map_err(|error| {
        let end = log.iter().position(|byte| *byte == 0).unwrap_or(log.len());
        io::Error::new(
            error.kind(),
            format!("{error}: {}", String::from_utf8_lossy(&log[..end]).trim()),
        )
    })?;
    let program = owned(loaded)?;
    let _ = std::fs::remove_file(pin);
    let name = path(pin)?;
    let mut attributes = ObjectAttributes {
        pathname: name.as_ptr() as u64,
        bpf_fd: program.as_raw_fd() as u32,
        file_flags: 0,
    };
    bpf(BPF_OBJ_PIN, &mut attributes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jumps_stay_inside_the_program() {
        let program = background_program(3, 4);
        assert_eq!(program.len(), 29);
        assert_eq!(program.last().map(|last| last.code), Some(EXIT));
        for (at, item) in program.iter().enumerate() {
            if matches!(item.code, JUMP_EQUAL | JUMP_NOT_EQUAL | JUMP_LESS) {
                let target = at as i64 + 1 + i64::from(item.offset);
                assert!(
                    (0..program.len() as i64).contains(&target),
                    "jump at {at} leaves the program"
                );
                assert_ne!(
                    program[target as usize].code, 0,
                    "jump at {at} lands inside a wide load"
                );
            }
        }
        for item in program.iter().filter(|item| item.code == LOAD_WORD) {
            assert_eq!(item.immediate, 0, "loads use the offset field");
        }
        assert_eq!(
            program[22].offset, 4,
            "the rule field follows the interface field"
        );
        let uids = uid_program(5);
        assert_eq!(uids[7].code, JUMP_EQUAL);
        assert_eq!(7 + 1 + uids[7].offset as usize, 10);
        assert_eq!((uids[8].immediate, uids[10].immediate), (1, 0));
        assert_eq!(
            (program[25].code, program[25].immediate),
            (MOV64_IMMEDIATE, 1)
        );
        assert_eq!(
            (program[27].code, program[27].immediate),
            (MOV64_IMMEDIATE, 0)
        );
    }
}
