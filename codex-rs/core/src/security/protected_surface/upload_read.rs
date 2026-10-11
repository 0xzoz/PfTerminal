//! #519: a Codex Apps upload read that runs outside the sandbox (profiles
//! with full write access) under the protected-path rules. Checking a path
//! and then reading it is a race: a link swapped in between makes the read
//! open a denied file. The file is opened once, without following a link at
//! its last component, what was opened is checked where the kernel says it
//! is, and the bytes come from that same handle.

use super::read_denials::readable_under;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use std::io;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

/// The contents of the file at `path` (at most `limit` bytes), when it is
/// readable under `policy` both before it is opened and as opened.
pub(crate) fn read_checked(
    policy: &FileSystemSandboxPolicy,
    path: &Path,
    cwd: &Path,
    limit: u64,
) -> io::Result<Vec<u8>> {
    if !readable_under(policy, path, cwd) {
        return Err(blocked());
    }
    read_opened(policy, &std::fs::canonicalize(path)?, cwd, limit)
}

/// Opens `canonical` (which a swapped folder may now redirect) and reads it
/// when what was opened is readable.
fn read_opened(
    policy: &FileSystemSandboxPolicy,
    canonical: &Path,
    cwd: &Path,
    limit: u64,
) -> io::Result<Vec<u8>> {
    let path = canonical;
    let mut file = open_no_follow(canonical)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("path `{}` is not a file", path.display()),
        ));
    }
    let opened = opened_path(&file, canonical, &metadata)?;
    if !readable_under(policy, &opened, cwd) {
        return Err(blocked());
    }
    if metadata.len() > limit {
        return Err(too_large(path, metadata.len(), limit));
    }
    let mut contents = Vec::new();
    (&mut file).take(limit + 1).read_to_end(&mut contents)?;
    if contents.len() as u64 > limit {
        return Err(too_large(path, contents.len() as u64, limit));
    }
    Ok(contents)
}

fn blocked() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "reading this file is blocked by the protected-path rules",
    )
}

fn too_large(path: &Path, size: u64, limit: u64) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "file `{}` is too large: {size} bytes exceeds the limit of {limit} bytes",
            path.display()
        ),
    )
}

#[cfg(unix)]
fn open_no_follow(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        // A FIFO swapped in would block the open.
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(path)
}

#[cfg(not(unix))]
fn open_no_follow(path: &Path) -> io::Result<std::fs::File> {
    std::fs::File::open(path)
}

/// Where the opened file is. macOS and Linux ask the kernel; elsewhere the
/// checked path must still resolve to itself and name the opened file.
fn opened_path(
    file: &std::fs::File,
    canonical: &Path,
    metadata: &std::fs::Metadata,
) -> io::Result<PathBuf> {
    if let Some(path) = kernel_path(file)? {
        return Ok(path);
    }
    let again = std::fs::canonicalize(canonical)?;
    if again != canonical || !same_file(&std::fs::metadata(&again)?, metadata) {
        return Err(io::Error::other("the file changed while it was opened"));
    }
    Ok(again)
}

#[cfg(target_os = "macos")]
fn kernel_path(file: &std::fs::File) -> io::Result<Option<PathBuf>> {
    use std::ffi::CStr;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::AsRawFd;
    let mut buffer = vec![0 as libc::c_char; libc::PATH_MAX as usize];
    // SAFETY: `F_GETPATH` writes a NUL-terminated path of at most
    // `PATH_MAX` bytes into `buffer`, which holds that many.
    let result = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETPATH, buffer.as_mut_ptr()) };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success the buffer holds a NUL-terminated string.
    let path = unsafe { CStr::from_ptr(buffer.as_ptr()) };
    Ok(Some(PathBuf::from(std::ffi::OsStr::from_bytes(
        path.to_bytes(),
    ))))
}

#[cfg(target_os = "linux")]
fn kernel_path(file: &std::fs::File) -> io::Result<Option<PathBuf>> {
    use std::os::unix::io::AsRawFd;
    std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).map(Some)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn kernel_path(_file: &std::fs::File) -> io::Result<Option<PathBuf>> {
    Ok(None)
}

#[cfg(unix)]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    a.dev() == b.dev() && a.ino() == b.ino()
}

#[cfg(not(unix))]
fn same_file(a: &std::fs::Metadata, b: &std::fs::Metadata) -> bool {
    a.len() == b.len() && a.modified().ok() == b.modified().ok()
}

#[cfg(test)]
#[path = "upload_read_tests.rs"]
mod tests;
