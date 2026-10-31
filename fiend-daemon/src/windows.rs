#![allow(unsafe_code)]

use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::Duration;

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, GetTokenInformation, PROTECTED_DACL_SECURITY_INFORMATION, PSID,
    RevertToSelf, SECURITY_ATTRIBUTES, SetFileSecurityW, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER,
    TokenElevation, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_DELAY_UNTIL_REBOOT, MoveFileExW};
use windows_sys::Win32::System::Pipes::ImpersonateNamedPipeClient;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThread, OpenProcessToken, OpenThreadToken,
};

pub const SERVICE_NAME: &str = "fiend";
const SYSTEM_SID: &str = "S-1-5-18";

pub fn program_data() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
}

pub fn program_files() -> PathBuf {
    std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
}

pub fn store_dir() -> PathBuf {
    program_data().join("Fiend")
}

pub fn install_dir() -> PathBuf {
    program_files().join("Fiend")
}

pub fn helper_path() -> PathBuf {
    install_dir().join("fiend-daemon.exe")
}

pub fn wintun_path() -> PathBuf {
    install_dir().join("wintun.dll")
}

pub fn allowed_sid_path() -> PathBuf {
    store_dir().join("allowed-user.sid")
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub fn valid_sid(sid: &str) -> bool {
    sid.starts_with("S-1-")
        && sid
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-')
}

fn sid_string(sid: PSID) -> io::Result<String> {
    let mut text: *mut u16 = ptr::null_mut();
    let ok = unsafe { ConvertSidToStringSidW(sid, &mut text) };
    if ok == 0 || text.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut length = 0usize;
    while unsafe { *text.add(length) } != 0 {
        length += 1;
    }
    let result = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, length) });
    unsafe {
        LocalFree(text as _);
    }
    Ok(result)
}

fn token_sid(token: HANDLE) -> io::Result<String> {
    let mut length = 0u32;
    unsafe {
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut length);
    }
    if length == 0 {
        return Err(io::Error::last_os_error());
    }
    let words = (length as usize).div_ceil(std::mem::size_of::<usize>());
    let mut buffer = vec![0usize; words];
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buffer.as_mut_ptr() as *mut _,
            length,
            &mut length,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
    sid_string(user.User.Sid)
}

fn process_sid(process: HANDLE) -> io::Result<String> {
    let mut token: HANDLE = ptr::null_mut();
    let ok = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = token_sid(token);
    unsafe {
        CloseHandle(token);
    }
    result
}

pub fn current_user_sid() -> io::Result<String> {
    process_sid(unsafe { GetCurrentProcess() })
}

struct Impersonation;

impl Drop for Impersonation {
    fn drop(&mut self) {
        if unsafe { RevertToSelf() } == 0 {
            std::process::abort();
        }
    }
}

#[derive(Clone, Copy)]
pub struct PipeClient(usize);

impl PipeClient {
    pub fn of(pipe: &NamedPipeServer) -> Self {
        Self(pipe.as_raw_handle() as usize)
    }

    pub fn sid(self) -> io::Result<String> {
        let pipe = self.0 as HANDLE;
        if unsafe { ImpersonateNamedPipeClient(pipe) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let _revert = Impersonation;
        let mut token: HANDLE = ptr::null_mut();
        let ok = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let result = token_sid(token);
        unsafe {
            CloseHandle(token);
        }
        result
    }
}

pub fn is_elevated() -> io::Result<bool> {
    let mut token: HANDLE = ptr::null_mut();
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut length = 0u32;
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut length,
        )
    };
    unsafe {
        CloseHandle(token);
    }
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(elevation.TokenIsElevated != 0)
}

fn descriptor(sddl: &str) -> io::Result<*mut std::ffi::c_void> {
    let text = wide(sddl);
    let mut descriptor = ptr::null_mut();
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            text.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(descriptor)
}

fn set_dacl(path: &Path, sddl: &str) -> io::Result<()> {
    let descriptor = descriptor(sddl)?;
    let path = wide_path(path);
    let ok = unsafe {
        SetFileSecurityW(
            path.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    unsafe {
        LocalFree(descriptor);
    }
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn protected_sddl(sid: &str, directory: bool) -> String {
    let ace = |trustee: &str| {
        if directory {
            format!("(A;OICI;GA;;;{trustee})")
        } else {
            format!("(A;;GA;;;{trustee})")
        }
    };
    if sid == SYSTEM_SID {
        format!("D:P{}{}", ace("SY"), ace("BA"))
    } else {
        format!("D:P{}{}{}", ace("SY"), ace("BA"), ace(sid))
    }
}

pub fn allowed_sid() -> io::Result<String> {
    match std::fs::read_to_string(allowed_sid_path()) {
        Ok(sid) => {
            let sid = sid.trim().to_owned();
            if valid_sid(&sid) {
                Ok(sid)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "The allowed user SID file is not valid.",
                ))
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => current_user_sid(),
        Err(error) => Err(error),
    }
}

pub fn write_allowed_sid(sid: Option<&str>) -> io::Result<()> {
    let dir = store_dir();
    std::fs::create_dir_all(&dir)?;
    let sid = match sid {
        Some(sid) => sid.to_owned(),
        None => current_user_sid()?,
    };
    if !valid_sid(&sid) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "The user SID is not valid.",
        ));
    }
    set_dacl(&dir, "D:P(A;;GA;;;SY)(A;;GA;;;BA)")?;
    let path = allowed_sid_path();
    std::fs::write(&path, format!("{sid}\n"))?;
    set_dacl(&path, "D:P(A;;GA;;;SY)(A;;GA;;;BA)")
}

pub fn protect_store(path: &Path) -> io::Result<()> {
    let sid = current_user_sid()?;
    let directory = path.is_dir();
    set_dacl(path, &protected_sddl(&sid, directory))?;
    if directory {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            set_dacl(&path, &protected_sddl(&sid, path.is_dir()))?;
        }
    }
    Ok(())
}

pub fn remove_file(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => {
            let path = wide_path(path);
            let ok =
                unsafe { MoveFileExW(path.as_ptr(), ptr::null(), MOVEFILE_DELAY_UNTIL_REBOOT) };
            if ok == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }
    }
}

pub struct PipeSecurity {
    descriptor: *mut std::ffi::c_void,
    sid: String,
}

unsafe impl Send for PipeSecurity {}
unsafe impl Sync for PipeSecurity {}

impl PipeSecurity {
    pub fn new() -> io::Result<Self> {
        let sid = allowed_sid()?;
        if !valid_sid(&sid) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "The allowed user SID is not valid.",
            ));
        }
        let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})");
        Ok(Self {
            descriptor: descriptor(&sddl)?,
            sid,
        })
    }

    pub fn allows(&self, sid: &str) -> bool {
        sid.eq_ignore_ascii_case(&self.sid)
    }

    pub fn create(
        &self,
        options: &ServerOptions,
        name: &OsStr,
        first: bool,
    ) -> io::Result<NamedPipeServer> {
        let mut options = options.clone();
        options
            .first_pipe_instance(first)
            .reject_remote_clients(true);
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.descriptor,
            bInheritHandle: 0,
        };
        unsafe {
            options.create_with_security_attributes_raw(
                name,
                &mut attributes as *mut _ as *mut std::ffi::c_void,
            )
        }
    }
}

impl Drop for PipeSecurity {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.descriptor);
        }
    }
}

pub fn copy_file(source: &Path, destination: &Path) -> io::Result<()> {
    let mut last = None;
    for _ in 0..25 {
        match std::fs::copy(source, destination) {
            Ok(_) => return Ok(()),
            Err(error) => {
                last = Some(error);
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("cannot copy the file")))
}

pub fn install_wintun(source: &Path) -> io::Result<PathBuf> {
    let destination = wintun_path();
    if source != destination {
        copy_file(source, &destination)?;
    }
    Ok(destination)
}

pub fn source_wintun() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|parent| parent.join("wintun.dll"))
        .filter(|path| path.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sids_are_validated() {
        assert!(valid_sid("S-1-5-18"));
        assert!(valid_sid("S-1-5-21-100-200-300-1001"));
        assert!(!valid_sid(""));
        assert!(!valid_sid("S-1-5-21-abc"));
        assert!(!valid_sid("D:P(A;;GA;;;SY)"));
    }

    #[test]
    fn the_store_acl_has_the_user_and_inheritance() {
        let user = protected_sddl("S-1-5-21-1-2-3-1001", false);
        assert_eq!(
            user,
            "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;S-1-5-21-1-2-3-1001)"
        );
        let dir = protected_sddl("S-1-5-21-1-2-3-1001", true);
        assert!(dir.contains("(A;OICI;GA;;;SY)"));
        assert!(dir.contains("(A;OICI;GA;;;S-1-5-21-1-2-3-1001)"));
        let system = protected_sddl(SYSTEM_SID, false);
        assert_eq!(system, "D:P(A;;GA;;;SY)(A;;GA;;;BA)");
    }
}
