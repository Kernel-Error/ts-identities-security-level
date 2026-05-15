//! Persisted launch-geometry tunings, one entry per device name.
//!
//! Layout — JSON in `$XDG_CACHE_HOME/ts3level/tuning.json` (falling
//! back to `~/.cache/ts3level/tuning.json`):
//!
//! ```json
//! {
//!   "version": 1,
//!   "devices": {
//!     "NVIDIA GeForce RTX 4060 Ti": {
//!       "blocks_per_sm": 32,
//!       "threads_per_block": 256,
//!       "rate_mhps": 2480.0,
//!       "measured_at": "seconds-since-unix-epoch:1715760000"
//!     }
//!   }
//! }
//! ```
//!
//! The file is written atomically (temp + rename) so a concurrent crash
//! cannot leave a half-formed cache that the next run rejects.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

const CACHE_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedTuning {
    pub blocks_per_sm: u32,
    pub threads_per_block: u32,
    pub rate_mhps: f64,
    pub measured_at: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TuningCache {
    pub version: u32,
    pub devices: BTreeMap<String, CachedTuning>,
}

impl TuningCache {
    /// Default production load path: `cache_path()`.
    pub fn load() -> std::io::Result<Self> {
        Self::load_from(&cache_path())
    }

    /// Production update path: `cache_path()`.
    pub fn update(device_name: &str, entry: CachedTuning) -> std::io::Result<()> {
        Self::update_at(&cache_path(), device_name, entry)
    }

    /// Load a cache from an arbitrary path. Tests use this with a
    /// tempdir to stay free of `XDG_CACHE_HOME` and other process-wide
    /// state.
    pub fn load_from(path: &Path) -> std::io::Result<Self> {
        let bytes = std::fs::read(path)?;
        let parsed: TuningCache = serde_json::from_slice(&bytes).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("malformed tuning cache: {e}"),
            )
        })?;
        if parsed.version != CACHE_VERSION {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("tuning cache version {} != {CACHE_VERSION}", parsed.version),
            ));
        }
        Ok(parsed)
    }

    pub fn get(&self, device_name: &str) -> Option<&CachedTuning> {
        self.devices.get(device_name)
    }

    /// Idempotently update one entry at an explicit path. Reads the
    /// existing file (if any), rewrites it via `tempfile + rename` so a
    /// partial file can't end up on disk.
    pub fn update_at(path: &Path, device_name: &str, entry: CachedTuning) -> std::io::Result<()> {
        let mut cache = Self::load_from(path).unwrap_or_else(|_| Self {
            version: CACHE_VERSION,
            devices: BTreeMap::new(),
        });
        cache.devices.insert(device_name.to_owned(), entry);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let serialized = serde_json::to_vec_pretty(&cache).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("serialize cache: {e}"),
            )
        })?;

        let parent = path.parent().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "cache path has no parent")
        })?;
        let mut tmp = tempfile::Builder::new()
            .prefix(".tuning-")
            .suffix(".tmp")
            .tempfile_in(parent)?;
        tmp.write_all(&serialized)?;
        tmp.flush()?;
        tmp.as_file().sync_all()?;
        tmp.persist(path).map_err(|e| e.error)?;
        Ok(())
    }
}

fn cache_dir() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(xdg).join("ts3level");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".cache").join("ts3level");
    }
    PathBuf::from("/tmp/ts3level")
}

pub fn cache_path() -> PathBuf {
    cache_dir().join("tuning.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_via_update_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tuning.json");

        TuningCache::update_at(
            &path,
            "Test GPU",
            CachedTuning {
                blocks_per_sm: 24,
                threads_per_block: 256,
                rate_mhps: 1234.5,
                measured_at: "seconds-since-unix-epoch:1".into(),
            },
        )
        .unwrap();

        let loaded = TuningCache::load_from(&path).unwrap();
        let entry = loaded.get("Test GPU").expect("entry persisted");
        assert_eq!(entry.blocks_per_sm, 24);
        assert_eq!(entry.threads_per_block, 256);
        assert!((entry.rate_mhps - 1234.5).abs() < 0.01);
    }

    #[test]
    fn version_mismatch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tuning.json");
        std::fs::write(&path, br#"{"version": 999, "devices": {}}"#).unwrap();
        let err = TuningCache::load_from(&path).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn missing_cache_returns_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does-not-exist.json");
        let err = TuningCache::load_from(&path).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn update_writes_atomically() {
        // Three updates in a row leave only the final file behind, no
        // `.tuning-…tmp` artifacts.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tuning.json");

        for i in 0..3 {
            TuningCache::update_at(
                &path,
                "Atomic",
                CachedTuning {
                    blocks_per_sm: 16 + i,
                    threads_per_block: 128,
                    rate_mhps: 100.0 + i as f64,
                    measured_at: format!("seconds-since-unix-epoch:{i}"),
                },
            )
            .unwrap();
        }

        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(entries.len(), 1, "stale tempfiles left behind: {entries:?}");
        assert_eq!(entries[0], "tuning.json");
    }

    #[test]
    fn append_preserves_other_devices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tuning.json");

        TuningCache::update_at(
            &path,
            "GPU A",
            CachedTuning {
                blocks_per_sm: 16,
                threads_per_block: 128,
                rate_mhps: 100.0,
                measured_at: "1".into(),
            },
        )
        .unwrap();
        TuningCache::update_at(
            &path,
            "GPU B",
            CachedTuning {
                blocks_per_sm: 48,
                threads_per_block: 512,
                rate_mhps: 200.0,
                measured_at: "2".into(),
            },
        )
        .unwrap();

        let loaded = TuningCache::load_from(&path).unwrap();
        assert_eq!(loaded.devices.len(), 2);
        assert!(loaded.get("GPU A").is_some());
        assert!(loaded.get("GPU B").is_some());
    }
}
