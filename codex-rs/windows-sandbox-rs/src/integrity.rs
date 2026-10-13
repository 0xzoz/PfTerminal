//! Low integrity for the legacy (unelevated) backend (#158).
//!
//! A `WRITE_RESTRICTED` token does not restrict `FILE_DELETE_CHILD`, so a
//! parent directory that grants the user that right lets the sandbox delete or
//! move children outside every writable root. The mandatory integrity check
//! does cover it: a Low-integrity process cannot delete or move an object
//! through a parent labelled Medium (the default for unlabelled objects).
//! Legacy sessions therefore run at Low integrity, and their writable roots
//! carry an inheritable Low label so writes inside them keep working.

use crate::token::LocalSid;
use crate::winutil::to_wide;
use anyhow::Result;
use anyhow::anyhow;
use std::ffi::c_void;
use std::path::Path;
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Foundation::HLOCAL;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::ACE_HEADER;
use windows_sys::Win32::Security::ACL;
use windows_sys::Win32::Security::ACL_REVISION;
use windows_sys::Win32::Security::ACL_SIZE_INFORMATION;
use windows_sys::Win32::Security::AclSizeInformation;
use windows_sys::Win32::Security::AddMandatoryAce;
use windows_sys::Win32::Security::Authorization::GetNamedSecurityInfoW;
use windows_sys::Win32::Security::Authorization::SE_FILE_OBJECT;
use windows_sys::Win32::Security::Authorization::SE_WINDOW_OBJECT;
use windows_sys::Win32::Security::Authorization::SetNamedSecurityInfoW;
use windows_sys::Win32::Security::Authorization::SetSecurityInfo;
use windows_sys::Win32::Security::CONTAINER_INHERIT_ACE;
use windows_sys::Win32::Security::EqualSid;
use windows_sys::Win32::Security::GetAce;
use windows_sys::Win32::Security::GetAclInformation;
use windows_sys::Win32::Security::GetLengthSid;
use windows_sys::Win32::Security::GetSidSubAuthority;
use windows_sys::Win32::Security::GetSidSubAuthorityCount;
use windows_sys::Win32::Security::GetTokenInformation;
use windows_sys::Win32::Security::INHERIT_ONLY_ACE;
use windows_sys::Win32::Security::InitializeAcl;
use windows_sys::Win32::Security::LABEL_SECURITY_INFORMATION;
use windows_sys::Win32::Security::OBJECT_INHERIT_ACE;
use windows_sys::Win32::Security::SID_AND_ATTRIBUTES;
use windows_sys::Win32::Security::SetTokenInformation;
use windows_sys::Win32::Security::TOKEN_MANDATORY_LABEL;
use windows_sys::Win32::Security::TokenIntegrityLevel;
use windows_sys::Win32::Storage::FileSystem::READ_CONTROL;
use windows_sys::Win32::Storage::FileSystem::WRITE_OWNER;
use windows_sys::Win32::System::StationsAndDesktops::CloseDesktop;
use windows_sys::Win32::System::StationsAndDesktops::OpenDesktopW;
use windows_sys::Win32::System::SystemServices::SE_GROUP_INTEGRITY;
use windows_sys::Win32::System::SystemServices::SYSTEM_MANDATORY_LABEL_ACE_TYPE;
use windows_sys::Win32::System::SystemServices::SYSTEM_MANDATORY_LABEL_NO_WRITE_UP;

const LOW_INTEGRITY_SID: &str = "S-1-16-4096";
const MEDIUM_INTEGRITY_SID: &str = "S-1-16-8192";
const LOW_INTEGRITY_RID: u32 = 0x1000;

/// Lowers `h_token` to Low integrity.
///
/// # Safety
/// `h_token` must be a valid token handle opened with `TOKEN_ADJUST_DEFAULT`.
pub(crate) unsafe fn set_token_low_integrity(h_token: HANDLE) -> Result<()> {
    let low = LocalSid::from_string(LOW_INTEGRITY_SID)?;
    let label = TOKEN_MANDATORY_LABEL {
        Label: SID_AND_ATTRIBUTES {
            Sid: low.as_ptr(),
            Attributes: SE_GROUP_INTEGRITY as u32,
        },
    };
    let size = std::mem::size_of::<TOKEN_MANDATORY_LABEL>() as u32 + GetLengthSid(low.as_ptr());
    let ok = SetTokenInformation(
        h_token,
        TokenIntegrityLevel,
        &label as *const TOKEN_MANDATORY_LABEL as *const c_void,
        size,
    );
    if ok == 0 {
        return Err(anyhow!(
            "SetTokenInformation(TokenIntegrityLevel) failed: {}",
            GetLastError()
        ));
    }
    Ok(())
}

/// An ACL holding one no-write-up label ACE for `level` with `flags`.
struct LabelAcl {
    buffer: Vec<u64>,
    _level: LocalSid,
}

impl LabelAcl {
    unsafe fn new(level: &str, flags: u32) -> Result<Self> {
        let low = LocalSid::from_string(level)?;
        // ACL header + SYSTEM_MANDATORY_LABEL_ACE (header, mask, SID).
        let size = std::mem::size_of::<ACL>()
            + std::mem::size_of::<ACE_HEADER>()
            + std::mem::size_of::<u32>()
            + GetLengthSid(low.as_ptr()) as usize;
        let mut buffer = vec![0u64; size.div_ceil(8)];
        let acl = buffer.as_mut_ptr() as *mut ACL;
        if InitializeAcl(acl, (buffer.len() * 8) as u32, ACL_REVISION) == 0 {
            return Err(anyhow!("InitializeAcl failed: {}", GetLastError()));
        }
        if AddMandatoryAce(
            acl,
            ACL_REVISION,
            flags,
            SYSTEM_MANDATORY_LABEL_NO_WRITE_UP,
            low.as_ptr(),
        ) == 0
        {
            return Err(anyhow!("AddMandatoryAce failed: {}", GetLastError()));
        }
        Ok(Self {
            buffer,
            _level: low,
        })
    }

    fn as_ptr(&mut self) -> *mut ACL {
        self.buffer.as_mut_ptr() as *mut ACL
    }
}

/// Gives `path` (and, for a directory, everything below it) a Low
/// no-write-up label, so a Low-integrity sandbox can write there. Returns
/// false when the label is already in place.
///
/// # Safety
/// Calls Win32 security APIs on `path`.
pub(crate) unsafe fn label_writable_root_low(path: &Path) -> Result<bool> {
    let flags = if path.is_dir() {
        OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE
    } else {
        0
    };
    if has_low_label(path, flags)? {
        return Ok(false);
    }
    set_file_label(path, LOW_INTEGRITY_SID, flags)?;
    Ok(true)
}

/// Gives a protected path inside a writable root (and everything below it)
/// an explicit Medium label, so it does not inherit the root's Low label: no
/// other Low-integrity program may write `.git`, `.codex` or `.agents` there.
///
/// # Safety
/// Calls Win32 security APIs on `path`.
pub(crate) unsafe fn label_protected_path_medium(path: &Path) -> Result<()> {
    let flags = if path.is_dir() {
        OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE
    } else {
        0
    };
    set_file_label(path, MEDIUM_INTEGRITY_SID, flags)
}

unsafe fn set_file_label(path: &Path, level: &str, flags: u32) -> Result<()> {
    let mut acl = LabelAcl::new(level, flags)?;
    // Inheritable label ACEs propagate to existing children.
    let code = SetNamedSecurityInfoW(
        to_wide(path).as_ptr(),
        SE_FILE_OBJECT,
        LABEL_SECURITY_INFORMATION,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        std::ptr::null(),
        acl.as_ptr(),
    );
    if code != ERROR_SUCCESS {
        return Err(anyhow!(
            "SetNamedSecurityInfoW(label) failed for {}: {code}",
            path.display()
        ));
    }
    Ok(())
}

unsafe fn has_low_label(path: &Path, flags: u32) -> Result<bool> {
    let mut p_sd: *mut c_void = std::ptr::null_mut();
    let mut p_sacl: *mut ACL = std::ptr::null_mut();
    let code = GetNamedSecurityInfoW(
        to_wide(path).as_ptr(),
        SE_FILE_OBJECT,
        LABEL_SECURITY_INFORMATION,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        &mut p_sacl,
        &mut p_sd,
    );
    if code != ERROR_SUCCESS {
        return Err(anyhow!(
            "GetNamedSecurityInfoW(label) failed for {}: {code}",
            path.display()
        ));
    }
    let found = !p_sacl.is_null() && sacl_has_low_label(p_sacl, flags);
    if !p_sd.is_null() {
        LocalFree(p_sd as HLOCAL);
    }
    Ok(found)
}

unsafe fn sacl_has_low_label(p_sacl: *mut ACL, flags: u32) -> bool {
    let Ok(low) = LocalSid::from_string(LOW_INTEGRITY_SID) else {
        return false;
    };
    let mut info: ACL_SIZE_INFORMATION = std::mem::zeroed();
    if GetAclInformation(
        p_sacl,
        &mut info as *mut ACL_SIZE_INFORMATION as *mut c_void,
        std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
        AclSizeInformation,
    ) == 0
    {
        return false;
    }
    for i in 0..info.AceCount {
        let mut p_ace: *mut c_void = std::ptr::null_mut();
        if GetAce(p_sacl, i, &mut p_ace) == 0 {
            continue;
        }
        let hdr = &*(p_ace as *const ACE_HEADER);
        let ace_flags = u32::from(hdr.AceFlags);
        if u32::from(hdr.AceType) != SYSTEM_MANDATORY_LABEL_ACE_TYPE
            || ace_flags & INHERIT_ONLY_ACE != 0
            || ace_flags & flags != flags
        {
            continue;
        }
        let mask = *((p_ace as usize + std::mem::size_of::<ACE_HEADER>()) as *const u32);
        let sid = (p_ace as usize + std::mem::size_of::<ACE_HEADER>() + std::mem::size_of::<u32>())
            as *mut c_void;
        if mask & SYSTEM_MANDATORY_LABEL_NO_WRITE_UP != 0 && EqualSid(sid, low.as_ptr()) != 0 {
            return true;
        }
    }
    false
}

/// Labels a private desktop Low so a Low-integrity sandbox can attach to it.
///
/// # Safety
/// `handle` must be a desktop handle opened with `WRITE_OWNER`.
pub(crate) unsafe fn label_window_object_low(handle: HANDLE) -> Result<()> {
    let mut acl = LabelAcl::new(LOW_INTEGRITY_SID, /*flags*/ 0)?;
    let code = SetSecurityInfo(
        handle,
        SE_WINDOW_OBJECT,
        LABEL_SECURITY_INFORMATION,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        std::ptr::null(),
        acl.as_ptr(),
    );
    if code != ERROR_SUCCESS {
        return Err(anyhow!("SetSecurityInfo(label) failed for desktop: {code}"));
    }
    Ok(())
}

/// Labels this process's desktop `name` (of its own, non-interactive window
/// station) Low, for a Low-integrity command started on it.
pub(crate) fn label_desktop_low(name: &str) -> Result<()> {
    let wide = to_wide(name);
    // SAFETY: opens a desktop of this process's window station; closed below.
    let handle = unsafe { OpenDesktopW(wide.as_ptr(), 0, 0, READ_CONTROL | WRITE_OWNER) };
    if handle == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: `handle` was opened with WRITE_OWNER above.
    let result = unsafe { label_window_object_low(handle) };
    // SAFETY: opened above.
    unsafe {
        CloseDesktop(handle);
    }
    result
}

/// Whether `h_token` runs at Low integrity or below.
pub(crate) fn token_is_low_integrity(h_token: HANDLE) -> bool {
    let mut buffer = [0u64; 16];
    let mut needed = 0u32;
    // SAFETY: the buffer outlives the call and its size is passed.
    let ok = unsafe {
        GetTokenInformation(
            h_token,
            TokenIntegrityLevel,
            buffer.as_mut_ptr() as *mut c_void,
            (buffer.len() * 8) as u32,
            &mut needed,
        )
    };
    if ok == 0 {
        return false;
    }
    // SAFETY: GetTokenInformation filled a TOKEN_MANDATORY_LABEL whose SID
    // points into `buffer`.
    unsafe {
        let label = &*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let count = *GetSidSubAuthorityCount(label.Label.Sid);
        if count == 0 {
            return false;
        }
        *GetSidSubAuthority(label.Label.Sid, u32::from(count) - 1) <= LOW_INTEGRITY_RID
    }
}
