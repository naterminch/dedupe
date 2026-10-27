use crate::cli::MatchMode;
use crate::scan::FileEntry;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug)]
pub struct MatchBucket {
    pub key: String,
    pub entries: Vec<FileEntry>,
}

#[must_use]
pub fn match_entries(
    entries: &[FileEntry],
    mode: MatchMode,
    fuzzy_threshold: f64,
) -> Vec<MatchBucket> {
    match mode {
        MatchMode::Size => {
            let mut map: BTreeMap<u64, Vec<FileEntry>> = BTreeMap::new();
            for e in entries {
                map.entry(e.size).or_default().push(e.clone());
            }
            map.into_iter()
                .filter(|(_, v)| v.len() >= 2)
                .map(|(size, entries)| MatchBucket {
                    key: format!("size:{size}"),
                    entries,
                })
                .collect()
        }
        MatchMode::Name => {
            let mut map: BTreeMap<String, Vec<FileEntry>> = BTreeMap::new();
            for e in entries {
                map.entry(name_key(&e.path)).or_default().push(e.clone());
            }
            map.into_iter()
                .filter(|(_, v)| v.len() >= 2)
                .map(|(key, entries)| MatchBucket { key, entries })
                .collect()
        }
        MatchMode::Fuzzy => fuzzy_buckets(entries, fuzzy_threshold),
        MatchMode::Content => Vec::new(),
    }
}

pub(super) fn name_key(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let lowered = stem.to_ascii_lowercase();
    let mut base = lowered.as_str();
    for marker in [" - copy", " copy", " - kopie", " kopie"] {
        if let Some(idx) = base.rfind(marker)
            && idx > 0
            && base[idx + marker.len()..]
                .trim()
                .chars()
                .all(|c| c.is_ascii_digit() || c == '(' || c == ')' || c == ' ')
        {
            base = &base[..idx];
        }
    }
    let mut normalized = String::new();
    let mut last_space = false;
    for c in base.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                normalized.push(lc);
            }
            last_space = false;
        } else if !last_space {
            normalized.push(' ');
            last_space = true;
        }
    }
    normalized.trim().to_string()
}

fn fuzzy_buckets(entries: &[FileEntry], threshold: f64) -> Vec<MatchBucket> {
    let threshold = (threshold / 100.0).clamp(0.0, 1.0);
    let mut keyed: Vec<(String, &FileEntry)> = entries
        .iter()
        .filter_map(|e| {
            let key = name_key(&e.path);
            if key.is_empty() { None } else { Some((key, e)) }
        })
        .collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.path.cmp(&b.1.path)));
    let mut clusters: Vec<(String, Vec<FileEntry>)> = Vec::new();
    for (key, entry) in keyed {
        let mut best: Option<(f64, usize)> = None;
        for (i, (rep, _)) in clusters.iter().enumerate() {
            let ratio = name_similarity(&key, rep);
            if ratio >= threshold && best.is_none_or(|(b, _)| ratio > b) {
                best = Some((ratio, i));
            }
        }
        match best {
            Some((_, i)) => clusters[i].1.push(entry.clone()),
            None => clusters.push((key, vec![entry.clone()])),
        }
    }
    clusters
        .into_iter()
        .filter(|(_, v)| v.len() >= 2)
        .map(|(key, entries)| MatchBucket {
            key: format!("fuzzy:{key}"),
            entries,
        })
        .collect()
}

/// Normalized Levenshtein similarity in 0..=1.
#[must_use]
pub fn name_similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let dist = prev[b.len()] as f64;
    1.0 - dist / a.len().max(b.len()) as f64
}
