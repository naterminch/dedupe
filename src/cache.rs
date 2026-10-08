//! Persistent fingerprint cache for the perceptual (near-duplicate) pass.
//!
//! Images and videos are fingerprinted once; later scans reuse the stored
//! fingerprint when a file's size AND modification time are unchanged. The
//! scan already knows both for every entry, so a cache lookup is a plain map
//! hit — no extra filesystem I/O. Repeat scans of the same folders therefore
//! skip all decoding / frame sampling for unchanged files.
//!
//! The cache is one binary file (bincode) per user. Default location:
//! `~/.dedupe/fingerprints.bin` (`%USERPROFILE%\.dedupe\...` on Windows,
//! `$HOME/.dedupe/...` elsewhere), overridable with the `DEDUPE_CACHE` env
//! var. Corrupt or version-mismatched files are treated as an empty cache,
//! and failed writes are non-fatal (the scan still succeeds).
//!
//! Stale entries for changed/deleted files are skipped for free by the
//! size+mtime key and pruned by a size cap at save time, so the file never
//! grows without bound.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Format version; bump to invalidate all previously stored fingerprints.
const CACHE_VERSION: u32 = 1;
/// Rough cap on stored entries; the file is pruned down to this at save time.
const MAX_ENTRIES: usize = 50_000;

/// Format version of the content-hash cache (independent from fingerprints).
const HASH_CACHE_VERSION: u32 = 1;
/// Files below this size are fast to hash and would only bloat the cache;
/// their hashes are recomputed every run (same trade-off as czkawka).
pub const HASH_CACHE_MIN_SIZE: u64 = 256 * 1024;

/// A stored perceptual fingerprint, mirroring the runtime types in `similar`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CacheFp {
    Image { w: u32, h: u32, hash: u64 },
    Video { w: u32, h: u32, duration_ms: u64, frames: Vec<u64> },
}

/// A cached fingerprint. The file path is the map key; `size` + `mtime_secs`
/// are the validity check against the current state of the file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    pub size: u64,
    pub mtime_secs: i64,
    pub fp: CacheFp,
}

/// A fingerprint computed during this run, to be stored when the run ends.
#[derive(Debug, Clone)]
pub struct CacheWrite {
    pub path: PathBuf,
    pub size: u64,
    pub mtime_secs: i64,
    pub fp: CacheFp,
}

#[derive(Debug, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    entries: Vec<(PathBuf, CacheEntry)>,
}

/// In-memory fingerprint cache, loaded once per run and saved at the end.
#[derive(Debug)]
pub struct FingerprintCache {
    path: Option<PathBuf>,
    map: HashMap<PathBuf, CacheEntry>,
    dirty: bool,
}

impl FingerprintCache {
    /// Load the cache from the default location (or `DEDUPE_CACHE`).
    /// Missing or unreadable files yield an empty cache, never an error.
    pub fn load() -> Self {
        Self::load_from(default_cache_path())
    }

    /// Load from an explicit path (also used by tests).
    pub fn load_from(path: PathBuf) -> Self {
        let map = match fs::read(&path) {
            Ok(bytes) => match bincode::deserialize::<CacheFile>(&bytes) {
                Ok(f) if f.version == CACHE_VERSION => f.entries.into_iter().collect(),
                _ => HashMap::new(),
            },
            Err(_) => HashMap::new(),
        };
        Self {
            path: Some(path),
            map,
            dirty: false,
        }
    }

    /// Look up a fingerprint valid for `(size, mtime_secs)`. Files whose
    /// mtime is unknown (`None`) are never served from the cache — the key
    /// could not be validated against the current state of the file.
    pub fn get(&self, path: &Path, size: u64, mtime_secs: Option<i64>) -> Option<&CacheFp> {
        let entry = self.map.get(path)?;
        let mtime = mtime_secs?;
        if entry.size == size && entry.mtime_secs == mtime {
            Some(&entry.fp)
        } else {
            None
        }
    }

    /// Store a newly computed fingerprint (marks the cache dirty).
    pub fn insert_write(&mut self, w: CacheWrite) {
        self.map.insert(
            w.path,
            CacheEntry {
                size: w.size,
                mtime_secs: w.mtime_secs,
                fp: w.fp,
            },
        );
        self.dirty = true;
    }

    /// Persist the cache atomically (temp file + rename). Best-effort by
    /// design: a failed write (read-only home dir, ...) must never fail the
    /// scan, and an untouched cache is not rewritten at all.
    pub fn save(&self) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let Some(path) = &self.path else {
            return Ok(());
        };
        let mut entries: Vec<(PathBuf, CacheEntry)> =
            self.map.iter().map(|(p, e)| (p.clone(), e.clone())).collect();
        if entries.len() > MAX_ENTRIES {
            entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            entries.truncate(MAX_ENTRIES);
        }
        let file = CacheFile {
            version: CACHE_VERSION,
            entries,
        };
        let bytes = bincode::serialize(&file).map_err(io::Error::other)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("bin.tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Default cache file location: `DEDUPE_CACHE` if set (non-empty), otherwise
/// under the user's home directory, otherwise the current directory.
pub fn default_cache_path() -> PathBuf {
    if let Some(p) = std::env::var_os("DEDUPE_CACHE")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home).join(".dedupe").join("fingerprints.bin");
    }
    PathBuf::from("fingerprints.bin")
}

/// A cached content hash (partial and/or full), mirroring `hashing`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HashCacheEntry {
    pub size: u64,
    pub mtime_secs: i64,
    pub algo: String,
    pub partial: Option<String>,
    pub full: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct HashCacheFile {
    version: u32,
    entries: Vec<(PathBuf, HashCacheEntry)>,
}

/// Persistent content-hash cache: the biggest repeat-scan win.
///
/// Unlike perceptual fingerprints (always cached), content hashes are only
/// stored for files at or above [`HASH_CACHE_MIN_SIZE`] — small files hash
/// faster than a cache round-trip is worth. Entries are keyed by path and
/// validated by size+mtime+algorithm, so stale results are never served.
#[derive(Debug)]
pub struct HashCache {
    path: Option<PathBuf>,
    map: HashMap<PathBuf, HashCacheEntry>,
    dirty: bool,
}

impl HashCache {
    /// Load from the default location (sibling of the fingerprint cache).
    pub fn load() -> Self {
        Self::load_from(default_hash_cache_path())
    }

    pub fn load_from(path: PathBuf) -> Self {
        let map = match fs::read(&path) {
            Ok(bytes) => match bincode::deserialize::<HashCacheFile>(&bytes) {
                Ok(f) if f.version == HASH_CACHE_VERSION => f.entries.into_iter().collect(),
                _ => HashMap::new(),
            },
            Err(_) => HashMap::new(),
        };
        Self {
            path: Some(path),
            map,
            dirty: false,
        }
    }

    /// Look up cached hashes valid for `(size, mtime_secs, algo)`.
    pub fn get(
        &self,
        path: &Path,
        size: u64,
        mtime_secs: Option<i64>,
        algo: &str,
    ) -> Option<&HashCacheEntry> {
        let entry = self.map.get(path)?;
        let mtime = mtime_secs?;
        if entry.size == size && entry.mtime_secs == mtime && entry.algo == algo {
            Some(entry)
        } else {
            None
        }
    }

    /// Store a newly computed hash. Files below [`HASH_CACHE_MIN_SIZE`] and
    /// entries without an mtime are not stored. Partial and full hashes
    /// merge: storing one never drops the other.
    pub fn insert(&mut self, path: PathBuf, size: u64, mtime_secs: Option<i64>, algo: String, partial: Option<String>, full: Option<String>) {
        let Some(mtime_secs) = mtime_secs else {
            return;
        };
        if size < HASH_CACHE_MIN_SIZE || (partial.is_none() && full.is_none()) {
            return;
        }
        let (partial, full) = match self.map.get(&path) {
            Some(e) if e.size == size && e.mtime_secs == mtime_secs && e.algo == algo => (
                partial.or_else(|| e.partial.clone()),
                full.or_else(|| e.full.clone()),
            ),
            _ => (partial, full),
        };
        self.map.insert(
            path,
            HashCacheEntry {
                size,
                mtime_secs,
                algo,
                partial,
                full,
            },
        );
        self.dirty = true;
    }

    /// Persist atomically (temp file + rename); best-effort, never fails scans.
    pub fn save(&self) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let Some(path) = &self.path else {
            return Ok(());
        };
        let mut entries: Vec<(PathBuf, HashCacheEntry)> =
            self.map.iter().map(|(p, e)| (p.clone(), e.clone())).collect();
        if entries.len() > MAX_ENTRIES {
            entries.sort_unstable_by(|a, b| a.0.cmp(&b.0));
            entries.truncate(MAX_ENTRIES);
        }
        let file = HashCacheFile {
            version: HASH_CACHE_VERSION,
            entries,
        };
        let bytes = bincode::serialize(&file).map_err(io::Error::other)?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("bin.tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Default content-hash cache location: next to the fingerprint cache.
pub fn default_hash_cache_path() -> PathBuf {
    if let Some(p) = std::env::var_os("DEDUPE_CACHE")
        && !p.is_empty()
    {
        let p = PathBuf::from(p);
        return match p.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent.join("hashes.bin"),
            _ => PathBuf::from("hashes.bin"),
        };
    }
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        return PathBuf::from(home).join(".dedupe").join("hashes.bin");
    }
    PathBuf::from("hashes.bin")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "dedupe-cache-{tag}-{}.bin",
            std::process::id()
        ))
    }

    #[test]
    fn roundtrip_and_invalidation_by_size_or_mtime() {
        let path = tmp_path("rt");
        let _ = fs::remove_file(&path);

        let p = PathBuf::from("C:\\media\\photo.png");
        {
            let mut c = FingerprintCache::load_from(path.clone());
            c.insert_write(CacheWrite {
                path: p.clone(),
                size: 100,
                mtime_secs: 5,
                fp: CacheFp::Image {
                    w: 9,
                    h: 8,
                    hash: 42,
                },
            });
            c.save().unwrap();
        }

        let c = FingerprintCache::load_from(path.clone());
        let expected = CacheFp::Image {
            w: 9,
            h: 8,
            hash: 42,
        };
        assert_eq!(c.get(&p, 100, Some(5)), Some(&expected));
        assert_eq!(c.get(&p, 101, Some(5)), None, "size changed -> miss");
        assert_eq!(c.get(&p, 100, Some(6)), None, "mtime changed -> miss");
        assert_eq!(c.get(&p, 100, None), None, "unknown mtime never hits");
        assert_eq!(c.get(&PathBuf::from("other.png"), 100, Some(5)), None);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn corrupt_or_unknown_version_is_empty() {
        let path = tmp_path("corrupt");

        fs::write(&path, b"not bincode").unwrap();
        let c = FingerprintCache::load_from(path.clone());
        assert_eq!(c.map.len(), 0);

        let file = CacheFile {
            version: CACHE_VERSION + 1,
            entries: vec![],
        };
        fs::write(&path, bincode::serialize(&file).unwrap()).unwrap();
        let c = FingerprintCache::load_from(path.clone());
        assert_eq!(c.map.len(), 0);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn untouched_cache_is_not_rewritten() {
        let path = tmp_path("dirty");
        let _ = fs::remove_file(&path);

        let mut c = FingerprintCache::load_from(path.clone());
        assert!(!c.dirty);
        c.save().unwrap();
        assert!(!path.exists(), "no writes, no file");

        c.insert_write(CacheWrite {
            path: PathBuf::from("x.mp4"),
            size: 1,
            mtime_secs: 1,
            fp: CacheFp::Video {
                w: 9,
                h: 8,
                duration_ms: 1000,
                frames: vec![1, 2, 3],
            },
        });
        c.save().unwrap();
        assert!(path.exists(), "dirty cache is persisted");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn hash_cache_roundtrip_and_invalidation() {
        let path = tmp_path("hash-rt");
        let _ = fs::remove_file(&path);
        let p = PathBuf::from("C:\\media\\big.bin");
        {
            let mut c = HashCache::load_from(path.clone());
            c.insert(
                p.clone(),
                HASH_CACHE_MIN_SIZE,
                Some(7),
                "blake3".into(),
                Some("partial".into()),
                Some("full".into()),
            );
            c.save().unwrap();
        }
        let c = HashCache::load_from(path.clone());
        let hit = c.get(&p, HASH_CACHE_MIN_SIZE, Some(7), "blake3").unwrap();
        assert_eq!(hit.full.as_deref(), Some("full"));
        assert_eq!(hit.partial.as_deref(), Some("partial"));
        assert!(c.get(&p, HASH_CACHE_MIN_SIZE + 1, Some(7), "blake3").is_none());
        assert!(c.get(&p, HASH_CACHE_MIN_SIZE, Some(8), "blake3").is_none());
        assert!(c.get(&p, HASH_CACHE_MIN_SIZE, Some(7), "sha256").is_none());
        assert!(c.get(&p, HASH_CACHE_MIN_SIZE, None, "blake3").is_none());
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn hash_cache_skips_small_and_mtimeless_files() {
        let path = tmp_path("hash-min");
        let _ = fs::remove_file(&path);
        let mut c = HashCache::load_from(path.clone());
        c.insert(
            PathBuf::from("tiny.bin"),
            HASH_CACHE_MIN_SIZE - 1,
            Some(1),
            "blake3".into(),
            Some("p".into()),
            Some("f".into()),
        );
        c.insert(
            PathBuf::from("nodate.bin"),
            HASH_CACHE_MIN_SIZE,
            None,
            "blake3".into(),
            Some("p".into()),
            Some("f".into()),
        );
        assert!(!c.dirty, "nothing storable, cache stays clean");
        c.save().unwrap();
        assert!(!path.exists());
        let _ = fs::remove_file(&path);
    }
}
