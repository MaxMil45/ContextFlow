//! Win32 named-pipe plumbing: identities, the locked-down security descriptor,
//! server instances and client connections.

use std::ffi::c_void;
use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, RawHandle};
use std::ptr::null_mut;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SDDL_REVISION_1, SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, RevertToSelf, TokenElevation, TokenUser, OWNER_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_ELEVATION, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientSessionId, GetNamedPipeServerSessionId,
    ImpersonateNamedPipeClient, WaitNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetCurrentThread, OpenProcessToken, OpenThreadToken,
};

use crate::Identity;

const ERROR_FILE_NOT_FOUND: i32 = 2;
const ERROR_PIPE_BUSY: i32 = 231;
const BUFFER: u32 = 64 * 1024;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Closes a kernel handle on drop.
struct OwnedHandle(HANDLE);

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        // SAFETY: the handle is valid and closed exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

/// Frees a `LocalAlloc`ed pointer on drop.
struct LocalBox(*mut c_void);

impl Drop for LocalBox {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from an API documented to LocalAlloc it.
            unsafe { LocalFree(self.0) };
        }
    }
}

fn sid_to_string(sid: PSID) -> io::Result<String> {
    let mut out = null_mut();
    // SAFETY: `sid` is a valid SID for the duration of the call; `out` is
    // LocalAlloced by the API and freed by `LocalBox`.
    if unsafe { ConvertSidToStringSidW(sid, &mut out) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _free = LocalBox(out.cast());
    // SAFETY: `out` is a NUL-terminated wide string.
    let len = (0..).take_while(|&i| unsafe { *out.add(i) } != 0).count();
    let slice = unsafe { std::slice::from_raw_parts(out, len) };
    Ok(String::from_utf16_lossy(slice))
}

fn token_user_sid(token: HANDLE) -> io::Result<String> {
    let mut needed = 0u32;
    // SAFETY: a size query with a null buffer.
    unsafe { GetTokenInformation(token, TokenUser, null_mut(), 0, &mut needed) };
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    // u64 storage keeps TOKEN_USER suitably aligned.
    let mut buf = vec![0u64; (needed as usize).div_ceil(8)];
    // SAFETY: `buf` holds at least `needed` bytes.
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            buf.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the API filled `buf` with a TOKEN_USER whose SID points into `buf`.
    let user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    sid_to_string(user.User.Sid)
}

/// Whether `token` runs with administrator rights ("Run as administrator",
/// or an account with UAC off).
fn token_elevated(token: HANDLE) -> io::Result<bool> {
    let mut info = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut len = 0u32;
    // SAFETY: `info` is exactly the size we pass.
    if unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut info as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(info.TokenIsElevated != 0)
}

/// Whether this process runs with administrator rights.
pub fn current_elevated() -> io::Result<bool> {
    let mut token = null_mut();
    // SAFETY: the pseudo-handle of the current process; `token` is closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = OwnedHandle(token);
    token_elevated(token.0)
}

pub fn current_identity() -> io::Result<Identity> {
    let mut token = null_mut();
    // SAFETY: the pseudo-handle of the current process; `token` is closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = OwnedHandle(token);
    let sid = token_user_sid(token.0)?;
    let mut session = 0;
    // SAFETY: plain out-parameter.
    if unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Identity { sid, session })
}

/// The security descriptor every pipe instance gets: owned by the user, a
/// protected DACL granting only the user access.
pub fn sddl_for(sid: &str) -> String {
    format!("O:{sid}D:P(A;;GA;;;{sid})")
}

struct SecurityDescriptor(LocalBox);

impl SecurityDescriptor {
    fn for_user(sid: &str) -> io::Result<Self> {
        let sddl = wide(&sddl_for(sid));
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        // SAFETY: `sddl` is NUL-terminated; `sd` is LocalAlloced by the API.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(LocalBox(sd)))
    }
}

/// The server end: creates pipe instances and waits for clients.
pub struct Listener {
    name: Vec<u16>,
    sd: SecurityDescriptor,
    /// The next instance, created ahead so a client never finds no instance.
    pending: Option<File>,
}

// SAFETY: the security descriptor is only read by CreateNamedPipeW.
unsafe impl Send for Listener {}

impl Listener {
    /// Create the first instance of `name`. Fails with `AddrInUse` if the name
    /// already exists, whoever made it.
    pub fn bind(name: &str, own: &Identity) -> io::Result<Self> {
        let mut listener = Self {
            name: wide(name),
            sd: SecurityDescriptor::for_user(&own.sid)?,
            pending: None,
        };
        listener.pending = Some(listener.instance(true).map_err(|e| {
            if e.kind() == io::ErrorKind::PermissionDenied {
                io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("the pipe {name} already exists (is an agent already running?)"),
                )
            } else {
                e
            }
        })?);
        Ok(listener)
    }

    fn instance(&self, first: bool) -> io::Result<File> {
        let sa = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.sd.0 .0,
            bInheritHandle: 0,
        };
        let open_mode = PIPE_ACCESS_DUPLEX
            | if first {
                FILE_FLAG_FIRST_PIPE_INSTANCE
            } else {
                0
            };
        // SAFETY: `name` is NUL-terminated and `sa` outlives the call.
        let handle = unsafe {
            CreateNamedPipeW(
                self.name.as_ptr(),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                PIPE_UNLIMITED_INSTANCES,
                BUFFER,
                BUFFER,
                0,
                &sa,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: a fresh, owned handle.
        Ok(unsafe { File::from_raw_handle(handle as RawHandle) })
    }

    /// Wait for the next client.
    pub fn accept(&mut self) -> io::Result<File> {
        let pipe = match self.pending.take() {
            Some(p) => p,
            None => self.instance(false)?,
        };
        // SAFETY: a valid pipe handle; synchronous call.
        let ok = unsafe { ConnectNamedPipe(pipe.as_raw_handle() as HANDLE, null_mut()) };
        // A client that connected between create and connect is fine too.
        if ok == 0 && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
            return Err(io::Error::last_os_error());
        }
        // Have the next instance ready before handing this one off.
        self.pending = self.instance(false).ok();
        Ok(pipe)
    }
}

/// The connected client's identity and whether it runs with administrator
/// rights. Call after reading at least one byte from
/// it (impersonation needs that). The client opened the pipe at identification
/// level, so this can query its token but never act as it.
pub fn client_identity(pipe: &File) -> io::Result<(Identity, bool)> {
    let handle = pipe.as_raw_handle() as HANDLE;
    // SAFETY: a connected server pipe handle.
    if unsafe { ImpersonateNamedPipeClient(handle) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut token = null_mut();
    // SAFETY: open the impersonation token, checked as ourselves.
    let opened = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) };
    let open_err = io::Error::last_os_error();
    // SAFETY: stop impersonating. If that ever failed this thread would keep
    // running as someone else, so there is no safe way to continue.
    if unsafe { RevertToSelf() } == 0 {
        std::process::abort();
    }
    if opened == 0 {
        return Err(open_err);
    }
    let token = OwnedHandle(token);
    let sid = token_user_sid(token.0)?;
    let elevated = token_elevated(token.0)?;
    let mut session = 0;
    // SAFETY: plain out-parameter.
    if unsafe { GetNamedPipeClientSessionId(handle, &mut session) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((Identity { sid, session }, elevated))
}

/// The identity of the pipe's server end: the owner of the pipe object and the
/// server's session. A pipe planted by another user is owned by that user.
pub fn server_identity(pipe: &File) -> io::Result<Identity> {
    let handle = pipe.as_raw_handle() as HANDLE;
    let mut owner: PSID = null_mut();
    let mut sd: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: `owner` points into `sd`, which is freed after we are done.
    let err = unsafe {
        GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut sd,
        )
    };
    if err != 0 {
        return Err(io::Error::from_raw_os_error(err as i32));
    }
    let _free = LocalBox(sd);
    let sid = sid_to_string(owner)?;
    let mut session = 0;
    // SAFETY: plain out-parameter.
    if unsafe { GetNamedPipeServerSessionId(handle, &mut session) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Identity { sid, session })
}

/// Why a client could not open the pipe.
pub enum OpenError {
    /// No such pipe: no agent is running.
    NotFound,
    /// Every instance stayed busy for the whole wait.
    Busy,
    Io(io::Error),
}

/// Open the client end, waiting up to `wait` if every instance is busy.
pub fn open(name: &str, wait: Duration) -> Result<File, OpenError> {
    let deadline = std::time::Instant::now() + wait;
    loop {
        match OpenOptions::new()
            .read(true)
            .write(true)
            // Lets the server identify us, never impersonate us.
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(name)
        {
            Ok(f) => return Ok(f),
            Err(e) if e.raw_os_error() == Some(ERROR_FILE_NOT_FOUND) => {
                return Err(OpenError::NotFound)
            }
            // Every instance is taken: wait for one, then race for it again
            // (another client may win it), until the deadline.
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) => {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    return Err(OpenError::Busy);
                }
                let ms = left.as_millis().clamp(1, u32::MAX as u128) as u32;
                let w = wide(name);
                // SAFETY: `w` is NUL-terminated. A timeout just loops to the
                // deadline check; a vanished pipe shows up as NotFound.
                unsafe { WaitNamedPipeW(w.as_ptr(), ms) };
            }
            Err(e) => return Err(OpenError::Io(e)),
        }
    }
}

/// The pipe's access list, read back from the kernel: whether it is protected
/// from inheritance, and each entry as (allow?, SID). Reading the entries
/// themselves, not their SDDL text, matters: SDDL abbreviates some accounts
/// (the built-in Administrator prints as `LA`), so text cannot be compared.
#[cfg(test)]
pub(crate) fn dacl_entries(pipe: &File) -> io::Result<(bool, Vec<(bool, String)>)> {
    use windows_sys::Win32::Security::{
        AclSizeInformation, GetAce, GetAclInformation, GetSecurityDescriptorControl,
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION,
        SE_DACL_PROTECTED,
    };
    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;

    let mut sd: PSECURITY_DESCRIPTOR = null_mut();
    let mut dacl: *mut ACL = null_mut();
    // SAFETY: `dacl` points into `sd`, which is freed when we are done.
    let err = unsafe {
        GetSecurityInfo(
            pipe.as_raw_handle() as HANDLE,
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut sd,
        )
    };
    if err != 0 {
        return Err(io::Error::from_raw_os_error(err as i32));
    }
    let _sd = LocalBox(sd);
    if dacl.is_null() {
        return Err(io::Error::other("the pipe has no DACL (everyone allowed)"));
    }
    let (mut control, mut revision) = (0u16, 0u32);
    // SAFETY: `sd` is a valid security descriptor.
    if unsafe { GetSecurityDescriptorControl(sd, &mut control, &mut revision) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: plain out-struct of the size we pass.
    let mut info: ACL_SIZE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe {
        GetAclInformation(
            dacl,
            (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
            size_of::<ACL_SIZE_INFORMATION>() as u32,
            AclSizeInformation,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let mut entries = Vec::new();
    for index in 0..info.AceCount {
        let mut ace: *mut c_void = null_mut();
        // SAFETY: `index` is below the ACL's entry count.
        if unsafe { GetAce(dacl, index, &mut ace) } == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: allow and deny entries share this layout: header, mask, SID.
        let ace = ace as *mut ACCESS_ALLOWED_ACE;
        let allow = unsafe { (*ace).Header.AceType } == ACCESS_ALLOWED_ACE_TYPE;
        let sid = unsafe { std::ptr::addr_of_mut!((*ace).SidStart) } as PSID;
        entries.push((allow, sid_to_string(sid)?));
    }
    Ok((control & SE_DACL_PROTECTED != 0, entries))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_name() -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        format!(
            r"\\.\pipe\ContextFlow.UnitTest.{}.{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        )
    }

    #[test]
    fn dacl_grants_only_the_user() {
        let me = current_identity().unwrap();
        let listener = Listener::bind(&unique_name(), &me).unwrap();
        let (protected, entries) = dacl_entries(listener.pending.as_ref().unwrap()).unwrap();
        // Protected (nothing inherited), and exactly one entry: allow, for us.
        assert!(protected, "the DACL must not inherit entries");
        assert_eq!(entries, [(true, me.sid.clone())]);
    }

    /// Later instances get the same access list as the first.
    #[test]
    fn every_instance_is_locked_down() {
        let me = current_identity().unwrap();
        let listener = Listener::bind(&unique_name(), &me).unwrap();
        let second = listener.instance(false).unwrap();
        let (protected, entries) = dacl_entries(&second).unwrap();
        assert!(protected);
        assert_eq!(entries, [(true, me.sid.clone())]);
    }

    #[test]
    fn owner_is_the_user_even_when_elevated() {
        let me = current_identity().unwrap();
        let name = unique_name();
        let _listener = Listener::bind(&name, &me).unwrap();
        let client = match open(&name, Duration::from_secs(1)) {
            Ok(f) => f,
            Err(_) => panic!("could not open our own pipe"),
        };
        assert_eq!(server_identity(&client).unwrap(), me);
    }

    #[test]
    fn sddl_shape() {
        assert_eq!(
            sddl_for("S-1-5-21-1"),
            "O:S-1-5-21-1D:P(A;;GA;;;S-1-5-21-1)"
        );
    }
}
