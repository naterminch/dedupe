use crate::cache::HashCache;
use crate::cli::HashAlgo;
use crate::scan::FileEntry;
use anyhow::Result;
use indicatif::ProgressBar;
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;

/// Bytes read from the start of a file for the cheap partial-hash pre-filter.
const PARTIAL_HEAD_BYTES: u64 = 32 * 1024;
/// Bytes read from the end of a file (appended to the head in one digest).
const PARTIAL_TAIL_BYTES: u64 = 32 * 1024;
/// Files at or below this size are digested whole by `partial`, so the
/// partial hash doubles as the full hash (no second read).
const PARTIAL_TOTAL_BYTES: u64 = PARTIAL_HEAD_BYTES + PARTIAL_TAIL_BYTES;
const IO_CHUNK: usize = 256 * 1024;

/// Wraps a hash algorithm and knows how to hash files, partially or fully.
pub struct HashEngine {
    algo: HashAlgo,
}

impl HashEngine {
    pub fn new(algo: HashAlgo) -> Self {
        Self { algo }
    }

    pub fn name(&self) -> &'static str {
        match self.algo {
            HashAlgo::Blake3 => "blake3",
            HashAlgo::Sha256 => "sha256",
            HashAlgo::Md5 => "md5",
        }
    }

    fn digest_bytes(&self, data: &[u8]) -> String {
        match self.algo {
            HashAlgo::Blake3 => blake3::hash(data).to_hex().to_string(),
            HashAlgo::Sha256 => {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(data);
                format!("{:x}", hasher.finalize())
            }
            HashAlgo::Md5 => {
                use md5::{Digest, Md5};
                let mut hasher = Md5::new();
                hasher.update(data);
                format!("{:x}", hasher.finalize())
            }
        }
    }

    fn hash_reader<R: Read>(&self, mut reader: R) -> io::Result<String> {
        let mut buf = [0u8; IO_CHUNK];
        match self.algo {
            HashAlgo::Blake3 => {
                let mut hasher = blake3::Hasher::new();
                loop {
                    let n = reader.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                Ok(hasher.finalize().to_hex().to_string())
            }
            HashAlgo::Sha256 => {
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                loop {
                    let n = reader.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                Ok(format!("{:x}", hasher.finalize()))
            }
            HashAlgo::Md5 => {
                use md5::{Digest, Md5};
                let mut hasher = Md5::new();
                loop {
                    let n = reader.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buf[..n]);
                }
                Ok(format!("{:x}", hasher.finalize()))
            }
        }
    }

    /// Cheap pre-filter hash: the first `PARTIAL_HEAD_BYTES` plus the last
    /// `PARTIAL_TAIL_BYTES` in a single digest (czkawka-style head+tail, so
    /// files sharing only a header still separate early). Files at or below
    /// `PARTIAL_TOTAL_BYTES` are digested whole, making this identical to
    /// [`HashEngine::full`] for them.
    pub fn partial(&self, path: &Path, size: u64) -> io::Result<String> {
        let mut file = File::open(path)?;
        if size <= PARTIAL_TOTAL_BYTES {
            let mut data = Vec::with_capacity(size.min(1024 * 1024) as usize);
            file.take(size).read_to_end(&mut data)?;
            return Ok(self.digest_bytes(&data));
        }
        let mut buf = vec![0u8; PARTIAL_TOTAL_BYTES as usize];
        let head = read_filling(&mut file, &mut buf[..PARTIAL_HEAD_BYTES as usize])?;
        file.seek(SeekFrom::Start(size.saturating_sub(PARTIAL_TAIL_BYTES)))?;
        let tail_start = PARTIAL_HEAD_BYTES as usize;
        let tail = read_filling(&mut file, &mut buf[tail_start..])?;
        Ok(self.digest_bytes(&buf[..head + tail]))
    }

    /// Hash the entire file.
    pub fn full(&self, path: &Path) -> io::Result<String> {
        self.hash_reader(BufReader::new(File::open(path)?))
    }
}

/// Fill `buf` (short reads and interrupts tolerated); returns bytes read.
fn read_filling(file: &mut File, buf: &mut [u8]) -> io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match file.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}

/// One set of files proven (by full hash) to be identical.
#[derive(Debug)]
pub struct DuplicateGroup {
    pub hash: String,
    pub members: Vec<FileEntry>,
}

/// Drop hard-linked paths from a same-size group, keeping the first name
/// (czkawka does the same: two names for one inode are not two copies, and
/// deleting one would free no space). Pairwise, so pathologically large
/// groups skip filtering rather than going quadratic.
fn filter_hard_links(mut group: Vec<FileEntry>, skipped: &mut u64) -> Vec<FileEntry> {
    if group.len() > 2000 {
        return group;
    }
    let mut kept: Vec<FileEntry> = Vec::with_capacity(group.len());
    for entry in group.drain(..) {
        let is_dup = kept
            .iter()
            .any(|k: &FileEntry| same_file::is_same_file(&k.path, &entry.path).unwrap_or(false));
        if is_dup {
            *skipped += 1;
        } else {
            kept.push(entry);
        }
    }
    kept
}
/// The full detection pipeline:
/// 1. group candidate files by size (hard links filtered: same inode counts once),
/// 2. partial-hash the candidates in parallel and keep only same-partial groups,
/// 3. full-hash the survivors in parallel; identical full hashes are duplicates.
///
/// `hash_cache` (when given) is consulted before any I/O and updated with
/// newly computed hashes: repeat scans of unchanged files skip both hash
/// stages entirely. The lock is held only for map probes, never for hashing.
///
/// Returns the groups plus the number of hard-linked paths skipped.
pub fn find_duplicate_groups(
    entries: Vec<FileEntry>,
    engine: &HashEngine,
    hash_cache: Option<&mut HashCache>,
    progress: Option<&ProgressBar>,
) -> Result<(Vec<DuplicateGroup>, u64)> {
    let algo = engine.name().to_string();
    let cache = hash_cache.map(Mutex::new);

    let mut by_size: HashMap<u64, Vec<FileEntry>> = HashMap::new();
    for entry in entries {
        by_size.entry(entry.size).or_default().push(entry);
    }
    let mut hardlinks_skipped = 0u64;
    let candidate_groups: Vec<Vec<FileEntry>> = by_size
        .into_values()
        .filter(|g| g.len() >= 2)
        .map(|g| filter_hard_links(g, &mut hardlinks_skipped))
        .filter(|g| g.len() >= 2)
        .collect();

    let total_candidates: usize = candidate_groups.iter().map(Vec::len).sum();
    if let Some(bar) = progress {
        // Two ticks per candidate at most (partial + full); cached and
        // pruned files tick fewer, so position never exceeds length.
        bar.set_length(2 * total_candidates as u64);
        bar.set_message("hashing");
    }

    // Stages 2+3 run per size-group (groups are independent): seed buckets
    // from the cache, partial-hash the rest, then full-hash survivors.
    // Cached full hashes merge with freshly computed ones, so a cached file
    // still meets a newly-hashed partner.
    let groups: Vec<DuplicateGroup> = candidate_groups
        .par_iter()
        .flat_map(|group| {
            let mut seeded_full: HashMap<String, Vec<FileEntry>> = HashMap::new();
            let mut by_partial: HashMap<String, Vec<FileEntry>> = HashMap::new();
            for entry in group {
                let hit = cache.as_ref().and_then(|m| m.lock().ok()).and_then(|c| {
                    c.get(&entry.path, entry.size, entry.mtime_secs, &algo)
                        .map(|e| (e.partial.clone(), e.full.clone()))
                });
                match hit {
                    Some((_, Some(full))) => {
                        seeded_full.entry(full).or_default().push(entry.clone());
                    }
                    Some((Some(partial), None)) => {
                        by_partial.entry(partial).or_default().push(entry.clone());
                    }
                    _ => {
                        if let Ok(p) = engine.partial(&entry.path, entry.size) {
                            if let Some(m) = cache.as_ref()
                                && let Ok(mut c) = m.lock()
                            {
                                c.insert(
                                    entry.path.clone(),
                                    entry.size,
                                    entry.mtime_secs,
                                    algo.clone(),
                                    Some(p.clone()),
                                    None,
                                );
                            }
                            by_partial.entry(p).or_default().push(entry.clone());
                        }
                    }
                }
                if let Some(bar) = progress {
                    bar.inc(1);
                }
            }

            let mut by_full: HashMap<String, Vec<FileEntry>> = seeded_full;
            for (partial, members) in by_partial {
                if members.len() < 2 {
                    continue;
                }
                for entry in members {
                    // Small files were digested whole by `partial`: the
                    // bucket key already is the full content hash.
                    let full = if entry.size <= PARTIAL_TOTAL_BYTES {
                        Some(partial.clone())
                    } else {
                        cache
                            .as_ref()
                            .and_then(|m| m.lock().ok())
                            .and_then(|c| {
                                c.get(&entry.path, entry.size, entry.mtime_secs, &algo)
                                    .and_then(|e| e.full.clone())
                            })
                            .or_else(|| match engine.full(&entry.path) {
                                Ok(h) => {
                                    if let Some(m) = cache.as_ref()
                                        && let Ok(mut c) = m.lock()
                                    {
                                        c.insert(
                                            entry.path.clone(),
                                            entry.size,
                                            entry.mtime_secs,
                                            algo.clone(),
                                            None,
                                            Some(h.clone()),
                                        );
                                    }
                                    Some(h)
                                }
                                Err(_) => None,
                            })
                    };
                    if let Some(h) = full {
                        by_full.entry(h).or_default().push(entry);
                    }
                    if let Some(bar) = progress {
                        bar.inc(1);
                    }
                }
            }
            by_full
                .into_iter()
                .filter(|(_, files)| files.len() >= 2)
                .map(|(hash, files)| DuplicateGroup { hash, members: files })
                .collect::<Vec<_>>()
        })
        .collect();

    if let Some(bar) = progress {
        bar.finish_and_clear();
    }
    Ok((groups, hardlinks_skipped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("dedupe-hash-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(path: std::path::PathBuf, size: u64) -> FileEntry {
        FileEntry {
            path,
            size,
            mtime_secs: None,
        }
    }

    #[test]
    fn pipeline_finds_exact_duplicates_only() {
        let dir = tmpdir("pipeline");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        let c = dir.join("c.txt");
        fs::write(&a, b"same content here").unwrap();
        fs::write(&b, b"same content here").unwrap();
        fs::write(&c, b"different content!").unwrap();
        // Bigger file sharing only the prefix must NOT match a.
        let d = dir.join("d.txt");
        fs::write(&d, b"same content here but longer and different").unwrap();

        let entries = vec![
            entry(a.clone(), fs::metadata(&a).unwrap().len()),
            entry(b.clone(), fs::metadata(&b).unwrap().len()),
            entry(c.clone(), fs::metadata(&c).unwrap().len()),
            entry(d.clone(), fs::metadata(&d).unwrap().len()),
        ];
        let engine = HashEngine::new(HashAlgo::Blake3);
        let (groups, _) = find_duplicate_groups(entries, &engine, None, None).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 2);
        let mut paths: Vec<&std::path::Path> = groups[0].members.iter().map(|m| m.path.as_path()).collect();
        paths.sort();
        assert_eq!(paths, vec![a.as_path(), b.as_path()]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_hash_equals_full_for_small_files() {
        let dir = tmpdir("partial");
        let f = dir.join("small.txt");
        fs::write(&f, b"tiny").unwrap();
        let engine = HashEngine::new(HashAlgo::Blake3);
        assert_eq!(
            engine.partial(&f, 4).unwrap(),
            engine.full(&f).unwrap(),
            "tiny files are digested whole, so partial == full"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_covers_head_and_tail() {
        // Two large files sharing the head but differing in the tail must
        // separate at the partial stage (head-only hashing would group them).
        let dir = tmpdir("headtail");
        let head = vec![7u8; 40 * 1024];
        let mut a_content = head.clone();
        a_content.extend(vec![1u8; 40 * 1024]);
        let mut b_content = head;
        b_content.extend(vec![2u8; 40 * 1024]);
        let a = dir.join("a.bin");
        let b = dir.join("b.bin");
        fs::write(&a, &a_content).unwrap();
        fs::write(&b, &b_content).unwrap();
        let engine = HashEngine::new(HashAlgo::Blake3);
        assert_ne!(
            engine.partial(&a, a_content.len() as u64).unwrap(),
            engine.partial(&b, b_content.len() as u64).unwrap()
        );
        let entries = vec![
            entry(a, a_content.len() as u64),
            entry(b, b_content.len() as u64),
        ];
        let (groups, _) = find_duplicate_groups(entries, &engine, None, None).unwrap();
        assert!(groups.is_empty(), "same head, different tail => no group");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn progress_bar_position_never_exceeds_length() {
        // Regression: every candidate survives partial hashing (identical
        // small files), which used to tick the bar in BOTH stages against a
        // single length — 8 + 8 = 16/8. The bar must reset between stages so
        // position never exceeds length.
        let dir = tmpdir("bar");
        let mut entries = Vec::new();
        for i in 0..8 {
            let f = dir.join(format!("f{i}.txt"));
            fs::write(&f, b"same content").unwrap();
            entries.push(entry(f, 12));
        }
        let engine = HashEngine::new(HashAlgo::Blake3);
        let bar = indicatif::ProgressBar::new(0);
        let (groups, _) = find_duplicate_groups(entries, &engine, None, Some(&bar)).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].members.len(), 8);
        assert!(
            bar.position() <= bar.length().unwrap_or(0),
            "bar overflowed: position {} > length {:?}",
            bar.position(),
            bar.length()
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
