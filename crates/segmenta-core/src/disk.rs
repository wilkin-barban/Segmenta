//! Positional writes, preallocation and finalization (OpenSpec 02 §4, 08 §7).
//!
//! The syscall layer is thin and the arithmetic is pure: `pwrite_all` and
//! `preallocate` take the actual syscall as a closure so the `EINTR`, short-write and
//! `ENOSPC`/`EOPNOTSUPP` paths are tested without a hostile filesystem.

use std::fs::{File, OpenOptions};
use std::os::unix::io::{AsRawFd, RawFd};
use std::path::{Path, PathBuf};

use crate::error::{CoreError, Result};

/// What the preallocation step actually did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreallocOutcome {
    /// `posix_fallocate` reserved the blocks.
    Fallocate,
    /// The filesystem cannot fallocate; the length was set and free space checked.
    Truncate {
        /// Free bytes reported by `fstatvfs` before truncating.
        free_bytes: u64,
    },
}

/// `pwrite_all` over an injectable write call: `(buf, offset) -> bytes written`.
pub fn pwrite_all_with<F>(buf: &[u8], offset: u64, mut write: F) -> std::io::Result<()>
where
    F: FnMut(&[u8], u64) -> std::io::Result<usize>,
{
    let mut done = 0usize;
    while done < buf.len() {
        match write(&buf[done..], offset + done as u64) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "pwrite returned 0 bytes",
                ));
            }
            Ok(n) => done += n,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// `pwrite_all` on a raw descriptor: repeats until the buffer is empty and retries
/// `EINTR`. The descriptor cursor is never read or moved.
pub fn pwrite_all_fd(fd: RawFd, buf: &[u8], offset: u64) -> std::io::Result<()> {
    pwrite_all_with(buf, offset, |chunk, at| {
        let off = libc::off_t::try_from(at)
            .map_err(|_| std::io::Error::from_raw_os_error(libc::EINVAL))?;
        // SAFETY: `fd` is owned by the caller for the duration of the call and `chunk`
        // points to `len` initialized bytes.
        let written =
            unsafe { libc::pwrite(fd, chunk.as_ptr() as *const libc::c_void, chunk.len(), off) };
        if written < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(written as usize)
        }
    })
}

fn is_unsupported(err: &std::io::Error) -> bool {
    matches!(
        err.raw_os_error(),
        Some(libc::EOPNOTSUPP) | Some(libc::ENOSYS) | Some(libc::EINVAL)
    )
}

/// Preallocation with the fallback rules of OpenSpec 08 §6:
/// `ENOSPC` is terminal, `EOPNOTSUPP`/`ENOSYS` falls back to `set_len` only if
/// `free_bytes >= len`.
pub fn preallocate_with<FA, FT>(
    len: u64,
    free_bytes: u64,
    fallocate: FA,
    set_len: FT,
) -> Result<PreallocOutcome>
where
    FA: FnOnce() -> std::io::Result<()>,
    FT: FnOnce() -> std::io::Result<()>,
{
    match fallocate() {
        Ok(()) => Ok(PreallocOutcome::Fallocate),
        Err(err) if err.raw_os_error() == Some(libc::ENOSPC) => Err(CoreError::DiskFull),
        Err(err) if is_unsupported(&err) => {
            if free_bytes < len {
                return Err(CoreError::DiskFull);
            }
            set_len()?;
            Ok(PreallocOutcome::Truncate { free_bytes })
        }
        Err(err) => Err(CoreError::from(err)),
    }
}

/// Free bytes on the filesystem that hosts `fd`.
fn free_bytes(fd: RawFd) -> std::io::Result<u64> {
    // SAFETY: `stat` is zeroed and the kernel fills it for a valid descriptor.
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::fstatvfs(fd, &mut stat) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

/// A destination file written through explicit offsets.
#[derive(Debug)]
pub struct DiskWriter {
    file: File,
    path: PathBuf,
}

impl DiskWriter {
    /// Create the destination, failing if it already exists.
    pub fn create_new(path: &Path, total: Option<u64>) -> Result<(Self, PreallocOutcome)> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)?;
        let mut outcome = PreallocOutcome::Truncate { free_bytes: 0 };
        if let Some(total) = total {
            let fd = file.as_raw_fd();
            outcome = preallocate_with(
                total,
                free_bytes(fd)?,
                || {
                    // `posix_fallocate` returns an error number instead of setting errno.
                    let rc = unsafe { libc::posix_fallocate(fd, 0, total as libc::off_t) };
                    if rc == 0 {
                        Ok(())
                    } else {
                        Err(std::io::Error::from_raw_os_error(rc))
                    }
                },
                || file.set_len(total),
            )?;
        }
        Ok((
            DiskWriter {
                file,
                path: path.to_path_buf(),
            },
            outcome,
        ))
    }

    /// Reopen an existing partial file for resume; never truncates existing bytes.
    pub fn open_resume(path: &Path, total: u64) -> Result<(Self, PreallocOutcome)> {
        let file = OpenOptions::new().read(true).write(true).open(path)?;
        let current = file.metadata()?.len();
        if current > total {
            // A file longer than the remote object cannot be a valid partial download.
            file.set_len(0)?;
            file.set_len(total)?;
        } else if current < total && total > 0 {
            // The fallback path may not have set the full length before the crash.
            file.set_len(total)?;
        }
        Ok((
            DiskWriter {
                file,
                path: path.to_path_buf(),
            },
            PreallocOutcome::Truncate { free_bytes: 0 },
        ))
    }

    /// Write the whole buffer at `offset`, retrying `EINTR` and short writes.
    pub fn pwrite_all(&self, buf: &[u8], offset: u64) -> Result<()> {
        pwrite_all_fd(self.file.as_raw_fd(), buf, offset)?;
        Ok(())
    }

    /// Checkpoint the data written so far before the database records it.
    pub fn sync_data(&self) -> Result<()> {
        self.file.sync_data()?;
        Ok(())
    }

    /// Flush data and metadata, used before publishing the file.
    pub fn sync_all(&self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }

    /// Reset the file after a stale-cache restart.
    pub fn truncate(&self) -> Result<()> {
        self.file.set_len(0)?;
        self.file.sync_all()?;
        Ok(())
    }

    /// Current file length in bytes.
    pub fn len(&self) -> Result<u64> {
        Ok(self.file.metadata()?.len())
    }

    /// Whether the file is empty.
    pub fn is_empty(&self) -> Result<bool> {
        Ok(self.len()? == 0)
    }

    /// Path this writer owns.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Run a closure with the raw descriptor (used for size checks in tests).
    pub fn with_fd<T>(&self, f: impl FnOnce(RawFd) -> T) -> T {
        f(self.file.as_raw_fd())
    }
}
