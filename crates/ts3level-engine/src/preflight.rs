//! Side-effect-free preflight checks. The CLI prints them to stderr and
//! exits with a stable code; the GUI shows them as a modal dialog. All
//! variants carry enough structured info for either frontend to render a
//! useful message after `gettext` translation.

use crate::device::DeviceInfo;
use crate::engine::{EngineError, HashEngine};
use std::path::{Path, PathBuf};
use thiserror::Error;
use ts3level_core::writer::probe_lock;
use ts3level_core::IdentityFile;

/// Successful preflight: identity is parsed, lockable, and the engine
/// has at least one device to bind to.
#[derive(Debug)]
pub struct PreflightReport {
    pub identity: IdentityFile,
    pub current_counter: u64,
    pub devices: Vec<DeviceInfo>,
}

#[derive(Debug, Error)]
pub enum PreflightError {
    #[error("identity file not found: {path:?}")]
    NotFound { path: PathBuf },

    #[error("identity file is not readable: {path:?} (your uid: {our_uid}, file: {file_uid}:{file_gid} mode {mode:o})")]
    NotReadable {
        path: PathBuf,
        our_uid: u32,
        file_uid: u32,
        file_gid: u32,
        mode: u32,
    },

    #[error("identity file is not writable: {path:?}")]
    NotWritable { path: PathBuf },

    #[error("parent directory is not writable: {path:?} (atomic rename needs write access there)")]
    ParentNotWritable { path: PathBuf },

    #[error("invalid identity file: {source}")]
    InvalidFile {
        path: PathBuf,
        #[source]
        source: ts3level_core::Error,
    },

    #[error("identity file is currently locked by another process: {path:?}")]
    Locked { path: PathBuf },

    #[error("not enough free disk space in {path:?}: have {available} bytes, need at least {required} for the .bak copy")]
    InsufficientSpace {
        path: PathBuf,
        available: u64,
        required: u64,
    },

    #[error("backend driver missing: {0}")]
    DriverMissing(String),

    #[error("no compute device found by backend `{backend}`")]
    NoDevice { backend: &'static str },

    #[error("device file permission problem: {detail}")]
    DevicePermission { detail: String },

    #[error("backend reported an unexpected error: {0}")]
    BackendOther(String),
}

/// Run every preflight check in order. Stops at the first failure so the
/// frontend can surface a single, actionable message.
pub fn run_preflight(
    identity_path: &Path,
    engine: &dyn HashEngine,
) -> Result<PreflightReport, PreflightError> {
    let identity = check_file_and_parse(identity_path)?;
    let current_counter = identity.counter();
    let devices = engine_preflight(engine)?;
    Ok(PreflightReport { identity, current_counter, devices })
}

fn check_file_and_parse(path: &Path) -> Result<IdentityFile, PreflightError> {
    use std::os::unix::fs::MetadataExt;

    let metadata = std::fs::metadata(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => PreflightError::NotFound { path: path.to_owned() },
        std::io::ErrorKind::PermissionDenied => PreflightError::NotReadable {
            path: path.to_owned(),
            our_uid: nix_uid(),
            file_uid: 0,
            file_gid: 0,
            mode: 0,
        },
        _ => PreflightError::InvalidFile {
            path: path.to_owned(),
            source: ts3level_core::Error::PlainIo(e),
        },
    })?;

    // Read perm: try to open. Surface a structured error when EACCES.
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(PreflightError::NotReadable {
                path: path.to_owned(),
                our_uid: nix_uid(),
                file_uid: metadata.uid(),
                file_gid: metadata.gid(),
                mode: metadata.mode() & 0o7777,
            });
        }
        Err(e) => {
            return Err(PreflightError::InvalidFile {
                path: path.to_owned(),
                source: ts3level_core::Error::PlainIo(e),
            });
        }
    };

    // Write perm probe: open in append mode does not modify content but
    // verifies we may write. Errors with EACCES → NotWritable.
    match std::fs::OpenOptions::new().write(true).open(path) {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(PreflightError::NotWritable { path: path.to_owned() });
        }
        Err(e) => {
            return Err(PreflightError::InvalidFile {
                path: path.to_owned(),
                source: ts3level_core::Error::PlainIo(e),
            });
        }
    }

    // Parent dir writable (needed for atomic rename).
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    match std::fs::OpenOptions::new()
        .read(true)
        .open(parent)
        .and_then(|_| {
            let probe = parent.join(".ts3level-write-probe");
            std::fs::File::create(&probe).map(|_| probe)
        }) {
        Ok(probe) => {
            let _ = std::fs::remove_file(probe);
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(PreflightError::ParentNotWritable { path: parent.to_owned() });
        }
        Err(_) => { /* non-fatal: continue, write_back will surface it */ }
    }

    let identity = IdentityFile::parse(&bytes).map_err(|source| PreflightError::InvalidFile {
        path: path.to_owned(),
        source,
    })?;

    // Lock probe.
    if let Err(ts3level_core::Error::Locked(p)) = probe_lock(path) {
        return Err(PreflightError::Locked { path: p });
    }

    // Disk space check.
    if let Some((available, required)) = disk_space_short(parent, bytes.len() as u64 * 2) {
        return Err(PreflightError::InsufficientSpace {
            path: parent.to_owned(),
            available,
            required,
        });
    }

    Ok(identity)
}

fn engine_preflight(engine: &dyn HashEngine) -> Result<Vec<DeviceInfo>, PreflightError> {
    match engine.enumerate() {
        Ok(v) if v.is_empty() => Err(PreflightError::NoDevice { backend: engine.kind() }),
        Ok(v) => Ok(v),
        Err(EngineError::DriverMissing(m)) => Err(PreflightError::DriverMissing(m)),
        Err(EngineError::NoDevice) => Err(PreflightError::NoDevice { backend: engine.kind() }),
        Err(EngineError::DevicePermission { path, reason }) => {
            Err(PreflightError::DevicePermission {
                detail: format!("{path:?}: {reason}"),
            })
        }
        Err(e) => Err(PreflightError::BackendOther(e.to_string())),
    }
}

/// Return `Some((available, required))` if free space is too low.
fn disk_space_short(path: &Path, required: u64) -> Option<(u64, u64)> {
    let stat = rustix::fs::statvfs(path).ok()?;
    let available = stat.f_bavail as u64 * stat.f_frsize as u64;
    if available < required {
        Some((available, required))
    } else {
        None
    }
}

fn nix_uid() -> u32 {
    rustix::process::geteuid().as_raw()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::MockEngine;

    const SAMPLE_INI: &str = "\
[Identity]
identity=\"42Vr4FEM/ERFubjCxz6qh/yTZapjpx4UmRSQ34gegxCbGAtXXN1VgICMSxGCzReDAEACFkCfh9hVxcDZH0GcX0BBgAaRghzJEwAVjdeIA44Ki9fYFwePnZpSVopXV5oDEdbFx9kXCtCd0NJUUR5OXFXMDZaV1hIY25tY21FQnhFa3dFRjJ6dDdSUklKY2pSU0Ixa3dSNHhnPT0=\"
";

    #[test]
    fn happy_path_yields_devices_and_parsed_identity() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("good.ini");
        std::fs::write(&p, SAMPLE_INI).unwrap();
        let eng = MockEngine::new();
        let report = run_preflight(&p, &eng).unwrap();
        assert_eq!(report.current_counter, 42);
        assert_eq!(report.devices.len(), 1);
    }

    #[test]
    fn nonexistent_file() {
        let eng = MockEngine::new();
        let err = run_preflight(Path::new("/tmp/does-not-exist-ts3.ini"), &eng).unwrap_err();
        assert!(matches!(err, PreflightError::NotFound { .. }), "{err:?}");
    }

    #[test]
    fn invalid_identity_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("bad.ini");
        std::fs::write(&p, "this is not a teamspeak identity\n").unwrap();
        let eng = MockEngine::new();
        let err = run_preflight(&p, &eng).unwrap_err();
        assert!(matches!(err, PreflightError::InvalidFile { .. }), "{err:?}");
    }

    #[test]
    fn locked_file() {
        use std::os::fd::AsFd;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("locked.ini");
        std::fs::write(&p, SAMPLE_INI).unwrap();
        let f = std::fs::OpenOptions::new().read(true).write(true).open(&p).unwrap();
        rustix::fs::flock(f.as_fd(), rustix::fs::FlockOperation::LockExclusive).unwrap();
        let eng = MockEngine::new();
        let err = run_preflight(&p, &eng).unwrap_err();
        assert!(matches!(err, PreflightError::Locked { .. }), "{err:?}");
        drop(f);
    }

    #[test]
    fn read_only_file_is_not_writable() {
        use std::os::unix::fs::PermissionsExt;
        // Skip when running as root: root ignores DAC perms.
        if rustix::process::geteuid().is_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ro.ini");
        std::fs::write(&p, SAMPLE_INI).unwrap();
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o444);
        std::fs::set_permissions(&p, perms).unwrap();
        let eng = MockEngine::new();
        let err = run_preflight(&p, &eng).unwrap_err();
        assert!(matches!(err, PreflightError::NotWritable { .. }), "{err:?}");
        // restore for cleanup
        let mut perms = std::fs::metadata(&p).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&p, perms).unwrap();
    }
}
