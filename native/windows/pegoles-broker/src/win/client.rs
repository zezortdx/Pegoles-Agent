//! Who is on the other end of the pipe.
//!
//! The broker serves exactly one program: `pegoles-vm-host.exe` from its
//! own install folder (Program Files, writable by administrators only),
//! running as a signed-in user. The user's SID scopes the guest socket to
//! them; their token is used to open files, so the broker never reaches
//! anything that user could not already read and write.

use std::path::Path;

use windows::core::PWSTR;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{
    GetTokenInformation, ImpersonateLoggedOnUser, RevertToSelf, TokenUser, TOKEN_DUPLICATE,
    TOKEN_IMPERSONATE, TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::System::Pipes::{GetNamedPipeClientProcessId, ImpersonateNamedPipeClient};
use windows::Win32::System::Threading::{
    GetCurrentThread, OpenProcess, OpenThreadToken, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, SHGetKnownFolderPath, KF_FLAG_DEFAULT};

use super::util::{from_pwstr, local_free, Owned};

pub struct Client {
    /// `S-1-5-21-…` (checked by `pegoles_broker_proto::is_user_sid`).
    pub sid: String,
    /// The user's `%LOCALAPPDATA%`, from their token (not the request).
    pub local_app_data: String,
    token: Owned,
}

impl Client {
    /// Run `f` as this user (file opens, VM access grants), then revert.
    pub fn as_user<T>(&self, f: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        // SAFETY: a valid impersonation token we own.
        unsafe { ImpersonateLoggedOnUser(self.token.raw()) }
            .map_err(|e| format!("cannot act as the user: {e}"))?;
        let result = f();
        // SAFETY: undo the impersonation on this thread.
        if unsafe { RevertToSelf() }.is_err() {
            // Never keep serving with a user's identity on this thread.
            std::process::abort();
        }
        result
    }
}

fn image_path(pid: u32) -> Result<String, String> {
    // SAFETY: limited-information access is enough to read the image path.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map(Owned)
        .map_err(|e| format!("cannot inspect the client: {e}"))?;
    let mut buffer = vec![0u16; 1024];
    let mut size = buffer.len() as u32;
    // SAFETY: buffer and size describe the same allocation.
    unsafe {
        QueryFullProcessImageNameW(
            process.raw(),
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut size,
        )
    }
    .map_err(|e| format!("cannot read the client's path: {e}"))?;
    Ok(String::from_utf16_lossy(&buffer[..size as usize]))
}

fn token_sid(token: HANDLE) -> Result<String, String> {
    let mut needed = 0u32;
    // SAFETY: size query with no buffer (fails with the size in `needed`).
    let _ = unsafe { GetTokenInformation(token, TokenUser, None, 0, &mut needed) };
    if needed == 0 || needed > 4096 {
        return Err("cannot read the client's user".into());
    }
    let mut buffer = vec![0u8; needed as usize];
    // SAFETY: buffer of exactly `needed` bytes.
    unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
    }
    .map_err(|e| format!("cannot read the client's user: {e}"))?;
    // SAFETY: the buffer now holds a TOKEN_USER (and the SID it points into).
    let user = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
    let mut text = PWSTR::null();
    // SAFETY: a valid SID from the token; the string is LocalAlloc'd.
    unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) }
        .map_err(|e| format!("cannot read the client's user: {e}"))?;
    // SAFETY: NUL-terminated string from the OS, freed once below.
    let sid = unsafe { from_pwstr(text) };
    unsafe { local_free(text.0.cast()) };
    Ok(sid)
}

fn local_app_data(token: HANDLE) -> Result<String, String> {
    // SAFETY: known folder lookup for the given user's token.
    let path =
        unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, Some(token)) }
            .map_err(|e| format!("cannot find the user's folder: {e}"))?;
    // SAFETY: NUL-terminated string from the shell, freed once below.
    let text = unsafe { from_pwstr(path) };
    unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(path.0 as *const _)) };
    Ok(text)
}

/// Identify the pipe's client, or refuse it.
pub fn identify(pipe: HANDLE, expected_client: &Path) -> Result<Client, String> {
    let mut pid = 0u32;
    // SAFETY: a connected server-side pipe handle.
    unsafe { GetNamedPipeClientProcessId(pipe, &mut pid) }
        .map_err(|e| format!("cannot identify the client: {e}"))?;
    let image = image_path(pid)?;
    let expected = expected_client.to_string_lossy();
    if !pegoles_broker_proto::same_windows_path(&expected, &image) {
        return Err(format!(
            "only {} may use the broker",
            pegoles_broker_proto::CLIENT_EXE
        ));
    }
    // The client's token, through impersonation (reverted right away).
    // SAFETY: a connected pipe; the client allowed impersonation.
    unsafe { ImpersonateNamedPipeClient(pipe) }
        .map_err(|e| format!("cannot identify the client's user: {e}"))?;
    let mut token = HANDLE::default();
    // SAFETY: the impersonated thread token, opened with our own identity.
    let opened = unsafe {
        OpenThreadToken(
            GetCurrentThread(),
            TOKEN_QUERY | TOKEN_IMPERSONATE | TOKEN_DUPLICATE,
            true,
            &mut token,
        )
    };
    // SAFETY: always revert before doing anything else.
    if unsafe { RevertToSelf() }.is_err() {
        std::process::abort();
    }
    opened.map_err(|e| format!("cannot identify the client's user: {e}"))?;
    let token = Owned(token);
    let sid = token_sid(token.raw())?;
    if !pegoles_broker_proto::is_user_sid(&sid) {
        return Err("the broker only serves signed-in user accounts".into());
    }
    let local_app_data = local_app_data(token.raw())?;
    Ok(Client {
        sid,
        local_app_data,
        token,
    })
}
