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
use std::io::Write;
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};

/// Persist `identity` back to `path`.
pub fn write_back(path: &Path, identity: &IdentityFile) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        Error::Io {
            path: path.to_owned(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no parent"),
        }
    })?;

    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|source| Error::Io { path: path.to_owned(), source })?;
    flock_exclusive(&f, true).map_err(|_| Error::Locked(path.to_owned()))?;

    let bak = backup_path(path);
    if !bak.exists() {
        // Best-effort: if copy fails after the file is locked, the user
        // would be surprised that the .bak vanishes; preserve the error.
        std::fs::copy(path, &bak).map_err(|source| Error::Io {
            path: bak.clone(),
            source,
        })?;
    }

    let mut tmp = tempfile::Builder::new()
        .prefix(".ts3level-")
        .suffix(".tmp")
        .tempfile_in(parent)
        .map_err(|source| Error::Io { path: parent.to_owned(), source })?;

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

    tmp.persist(path).map_err(|e| Error::Io {
        path: path.to_owned(),
        source: e.error,
    })?;

    // f drops here, releasing the flock on the previous inode (which is
    // now unlinked — the new file at `path` is a different inode).
    drop(f);
    Ok(())
}

/// Side-effect-free probe: return `Ok(())` if we can acquire an exclusive
/// lock on `path` right now, `Err(Error::Locked(_))` if someone else holds
/// it. The lock is released immediately.
pub fn probe_lock(path: &Path) -> Result<()> {
    let f = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|source| Error::Io { path: path.to_owned(), source })?;
    match flock_exclusive(&f, false) {
        Ok(()) => Ok(()),
        Err(_) => Err(Error::Locked(path.to_owned())),
    }
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
