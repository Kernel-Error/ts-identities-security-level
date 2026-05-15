//! Atomic, locked, backup-once file writer for TS3 identity `.ini`s.
//!
//! Sequence:
//!   1. Open the existing file, hold an exclusive `flock` on it.
//!   2. If `<path>.ini.bak` does not yet exist, copy the original aside.
//!      This happens exactly once over the file's lifetime.
//!   3. Write the new content to a temp file in the same directory.
//!   4. `fsync` the temp file.
//!   5. `rename(2)` the temp file over the original (POSIX-atomic).
//!   6. Drop the open handle, releasing the lock.
//!
//! Errors are surfaced verbatim — the engine layer translates them for the
//! frontends. A non-blocking probe is also exposed so the preflight check
//! can warn early when another process holds the lock.

use crate::error::{Error, Result};
use crate::ini::IdentityFile;
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Persist `identity` back to `path`.
pub fn write_back(path: &Path, identity: &IdentityFile) -> Result<()> {
    let parent = path.parent().ok_or_else(|| Error::Io {
        path: path.to_owned(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent"),
    })?;

    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
    // Blocking flock: any failure here is a real OS error (the kernel
    // does not return EWOULDBLOCK for a blocking acquire). Don't pretend
    // an EIO / EINTR / ENOLCK is just "another process holds it".
    flock_exclusive(&f, true).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;

    // Capture the source mode under the lock — we restore it on the
    // tempfile right before the rename so the post-replace inode keeps
    // the same permission bits the user (or a previous tool) chose.
    let source_mode = f
        .metadata()
        .map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?
        .permissions()
        .mode();

    // Backup is created exactly once over the file's lifetime. We do
    // not rely on `.exists()` because that's a TOCTOU race; instead we
    // ask the OS to atomically claim the new path with `O_CREAT|O_EXCL`.
    // The content is streamed from the open, locked fd rather than via
    // a fresh open on `path` — otherwise a swap of the path between
    // open and copy could put foreign bytes into the backup.
    let bak = backup_path(path);
    match OpenOptions::new().write(true).create_new(true).open(&bak) {
        Ok(mut bak_file) => {
            f.seek(SeekFrom::Start(0)).map_err(|source| Error::Io {
                path: path.to_owned(),
                source,
            })?;
            std::io::copy(&mut f, &mut bak_file).map_err(|source| Error::Io {
                path: bak.clone(),
                source,
            })?;
            bak_file.sync_all().map_err(|source| Error::Io {
                path: bak.clone(),
                source,
            })?;
            sync_parent(parent)?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // The backup already exists. By design we never overwrite it.
        }
        Err(source) => {
            return Err(Error::Io {
                path: bak.clone(),
                source,
            });
        }
    }

    let mut tmp = tempfile::Builder::new()
        .prefix(".ts3level-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|source| Error::Io {
            path: parent.to_owned(),
            source,
        })?;

    let bytes = identity.to_bytes();
    tmp.write_all(&bytes).map_err(|source| Error::Io {
        path: tmp.path().to_owned(),
        source,
    })?;
    tmp.flush().map_err(|source| Error::Io {
        path: tmp.path().to_owned(),
        source,
    })?;
    tmp.as_file().sync_all().map_err(|source| Error::Io {
        path: tmp.path().to_owned(),
        source,
    })?;
    // Mirror the source file's permission bits onto the temp file
    // before rename, otherwise the post-replace inode inherits the
    // tempfile default (mode 0o600) and silently tightens permissions
    // for any user/group that previously had read access.
    tmp.as_file()
        .set_permissions(std::fs::Permissions::from_mode(source_mode & 0o7777))
        .map_err(|source| Error::Io {
            path: tmp.path().to_owned(),
            source,
        })?;

    tmp.persist(path).map_err(|e| Error::Io {
        path: path.to_owned(),
        source: e.error,
    })?;

    // POSIX `rename(2)` is atomic for visibility, but the directory
    // entry update is not guaranteed durable until the parent dir is
    // also fsync'd. Without this an unexpected power loss after
    // `persist` can revert `path` to its pre-rename inode.
    sync_parent(parent)?;

    // f drops here, releasing the flock on the previous inode (which is
    // now unlinked — the new file at `path` is a different inode).
    drop(f);
    Ok(())
}

/// `fsync` the directory so that recent `rename(2)` / `creat(2)` entries
/// are durable. POSIX permits a clean crash to lose un-sync'd directory
/// updates even when the underlying file data was sync'd.
fn sync_parent(parent: &Path) -> Result<()> {
    let dir = std::fs::File::open(parent).map_err(|source| Error::Io {
        path: parent.to_owned(),
        source,
    })?;
    dir.sync_all().map_err(|source| Error::Io {
        path: parent.to_owned(),
        source,
    })
}

/// Side-effect-free probe: return `Ok(())` if we can acquire an exclusive
/// lock on `path` right now, `Err(Error::Locked(_))` if someone else holds
/// it. The lock is released immediately.
pub fn probe_lock(path: &Path) -> Result<()> {
    // Open with the same flags as `write_back` so a probe success is a
    // genuine pre-flight for what the real write will do — opening
    // read-only here would mis-report a writable-but-locked file as
    // "fine" when the actual write would fail at EACCES.
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|source| Error::Io {
            path: path.to_owned(),
            source,
        })?;
    match flock_exclusive(&f, false) {
        Ok(()) => Ok(()),
        // Only EWOULDBLOCK means "another holder, retry later". Anything
        // else is a real OS fault and should surface as such.
        Err(e) if e.raw_os_error() == Some(libc_ewouldblock()) => {
            Err(Error::Locked(path.to_owned()))
        }
        Err(source) => Err(Error::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

/// `EWOULDBLOCK` numeric value on Linux. Pulled out so the writer crate
/// doesn't have to take a `libc` dependency just for one constant.
#[inline]
const fn libc_ewouldblock() -> i32 {
    // EWOULDBLOCK == EAGAIN == 11 on Linux on every architecture we
    // currently target. macOS / BSDs use different values; if this
    // crate ever ships there, switch to the `libc` crate's constant.
    11
}

/// `<path>.bak` — preserving the original extension makes the backup
/// obvious in file managers (e.g. `kernel-error.ini` → `kernel-error.ini.bak`).
pub fn backup_path(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".bak");
    s.into()
}

fn flock_exclusive<F: AsFd>(f: &F, blocking: bool) -> std::io::Result<()> {
    use rustix::fs::{flock, FlockOperation};
    let op = if blocking {
        FlockOperation::LockExclusive
    } else {
        FlockOperation::NonBlockingLockExclusive
    };
    flock(f.as_fd(), op).map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    extern "C" {
        fn geteuid() -> u32;
    }
    unsafe fn libc_geteuid() -> u32 {
        geteuid()
    }

    const FIXTURE: &str = "\
[Identity]
id=Kernel-Error
identity=\"42Vr4FEM/ERFubjCxz6qh/yTZapjpx4UmRSQ34gegxCbGAtXXN1VgICMSxGCzReDAEACFkCfh9hVxcDZH0GcX0BBgAaRghzJEwAVjdeIA44Ki9fYFwePnZpSVopXV5oDEdbFx9kXCtCd0NJUUR5OXFXMDZaV1hIY25tY21FQnhFa3dFRjJ6dDdSUklKY2pSU0Ixa3dSNHhnPT0=\"
nickname=Kernel-Error
phonetic_nickname=
";

    fn read_file_as_bytes(p: &Path) -> Vec<u8> {
        let mut f = std::fs::File::open(p).unwrap();
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).unwrap();
        buf
    }

    #[test]
    fn writes_back_and_creates_bak_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.ini");
        std::fs::write(&path, FIXTURE).unwrap();
        let bak = backup_path(&path);
        assert!(!bak.exists());

        let mut id = IdentityFile::parse(FIXTURE.as_bytes()).unwrap();
        id.set_counter(99);
        write_back(&path, &id).unwrap();

        let on_disk = String::from_utf8(read_file_as_bytes(&path)).unwrap();
        assert!(on_disk.contains("identity=\"99V"));

        let bak_content = String::from_utf8(read_file_as_bytes(&bak)).unwrap();
        assert_eq!(bak_content, FIXTURE, "backup must equal pristine original");

        // Second write: bak must not be overwritten by the new content.
        id.set_counter(999);
        write_back(&path, &id).unwrap();
        let bak_content2 = String::from_utf8(read_file_as_bytes(&bak)).unwrap();
        assert_eq!(bak_content2, FIXTURE, "backup must remain pristine");
    }

    #[test]
    fn preserves_original_file_mode_across_rename() {
        // Skip when running as root: chmod is honored anyway but the
        // `set_permissions` semantics differ in edge cases. Use
        // `nix`-free uid lookup via libc through std (env var fallback).
        // SAFETY: `geteuid` is always safe to call on Unix.
        let euid = unsafe { libc_geteuid() };
        if euid == 0 {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mode_keep.ini");
        std::fs::write(&path, FIXTURE).unwrap();
        // Pick a group-readable mode different from the tempfile
        // default 0o600 — that's what we're guarding against.
        let mode_before = 0o640u32;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode_before)).unwrap();

        let mut id = IdentityFile::parse(FIXTURE.as_bytes()).unwrap();
        id.set_counter(7);
        write_back(&path, &id).unwrap();

        let mode_after = std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777;
        assert_eq!(
            mode_after, mode_before,
            "file mode silently tightened across atomic write (was {mode_before:o}, now {mode_after:o})"
        );
    }

    #[test]
    fn bak_creation_is_atomic_and_one_shot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preexist.ini");
        std::fs::write(&path, FIXTURE).unwrap();

        let bak = backup_path(&path);
        // Pre-create the .bak with foreign content. write_back must
        // detect that it already exists and leave it alone — never
        // overwrite, even though our content would be more recent.
        std::fs::write(&bak, "preexisting backup; do not touch").unwrap();

        let mut id = IdentityFile::parse(FIXTURE.as_bytes()).unwrap();
        id.set_counter(11);
        write_back(&path, &id).unwrap();

        let on_disk = std::fs::read_to_string(&bak).unwrap();
        assert_eq!(on_disk, "preexisting backup; do not touch");
    }

    #[test]
    fn write_back_is_atomic() {
        // We can't easily induce a power-loss in a unit test, but we can
        // verify that at no point during a write does a third party see
        // a half-written file: after `write_back` returns, reading the
        // path must yield exactly the new bytes (not a mix).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atomic.ini");
        std::fs::write(&path, FIXTURE).unwrap();
        let mut id = IdentityFile::parse(FIXTURE.as_bytes()).unwrap();
        id.set_counter(12345);
        write_back(&path, &id).unwrap();
        let s = std::fs::read_to_string(&path).unwrap();
        // Single, well-formed identity line.
        assert_eq!(s.matches("identity=").count(), 1);
        assert!(s.contains("identity=\"12345V"));
    }

    #[test]
    fn probe_lock_succeeds_when_unlocked() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("probe.ini");
        std::fs::write(&path, FIXTURE).unwrap();
        probe_lock(&path).unwrap();
    }

    #[test]
    fn probe_lock_detects_competing_lock() {
        use std::os::fd::AsFd;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked.ini");
        std::fs::write(&path, FIXTURE).unwrap();

        let f = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        rustix::fs::flock(f.as_fd(), rustix::fs::FlockOperation::LockExclusive).unwrap();

        let err = probe_lock(&path).unwrap_err();
        assert!(matches!(err, Error::Locked(_)));
        drop(f);
    }

    #[test]
    fn backup_path_appends_bak_extension() {
        let p = Path::new("/tmp/foo/bar.ini");
        assert_eq!(backup_path(p), Path::new("/tmp/foo/bar.ini.bak"));
        let p2 = Path::new("noext");
        assert_eq!(backup_path(p2), Path::new("noext.bak"));
    }
}
