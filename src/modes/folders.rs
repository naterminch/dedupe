use super::FindingGroup;
use super::walk::Tree;
use crate::cancel::CancelToken;
use crate::hashing::HashEngine;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

pub(super) fn duplicate_folders(
    tree: &Tree,
    engine: &HashEngine,
    cancel: &CancelToken,
) -> (Vec<FindingGroup>, Vec<String>) {
    let mut notes = Vec::new();
    let mut files_by_dir: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (i, file) in tree.files.iter().enumerate() {
        if let Some(parent) = file.path.parent() {
            files_by_dir
                .entry(parent.to_path_buf())
                .or_default()
                .push(i);
        }
    }
    let mut children: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
    for dir in &tree.dirs {
        if let Some(parent) = dir.parent() {
            children
                .entry(parent.to_path_buf())
                .or_default()
                .push(dir.clone());
        }
    }
    let mut dirs: Vec<PathBuf> = tree.dirs.clone();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
    let mut signatures: HashMap<PathBuf, String> = HashMap::new();
    let mut file_counts: HashMap<PathBuf, u64> = HashMap::new();
    let mut byte_counts: HashMap<PathBuf, u64> = HashMap::new();
    for dir in &dirs {
        if cancel.is_cancelled() {
            break;
        }
        let mut entries: Vec<String> = Vec::new();
        let mut count = 0u64;
        let mut bytes = 0u64;
        if let Some(files) = files_by_dir.get(dir) {
            for &i in files {
                let file = &tree.files[i];
                let name = file
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                match engine.full(&file.path) {
                    Ok(hash) => entries.push(format!("f:{name}\0{}\0{hash}", file.size)),
                    Err(err) => notes.push(format!("cannot hash {}: {err}", file.path.display())),
                }
                count += 1;
                bytes += file.size;
            }
        }
        if let Some(kids) = children.get(dir) {
            let mut kids = kids.clone();
            kids.sort();
            for kid in kids {
                if let Some(sig) = signatures.get(&kid) {
                    let name = kid
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    entries.push(format!("d:{name}\0{sig}"));
                    count += file_counts.get(&kid).copied().unwrap_or(0);
                    bytes += byte_counts.get(&kid).copied().unwrap_or(0);
                }
            }
        }
        entries.sort();
        let sig = blake3::hash(entries.join("\n").as_bytes())
            .to_hex()
            .to_string();
        signatures.insert(dir.clone(), sig);
        file_counts.insert(dir.clone(), count);
        byte_counts.insert(dir.clone(), bytes);
    }
    let mut by_sig: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for dir in &tree.dirs {
        if tree.roots.contains(dir) {
            continue;
        }
        let Some(sig) = signatures.get(dir) else {
            continue;
        };
        if file_counts.get(dir).copied().unwrap_or(0) == 0 {
            continue;
        }
        by_sig.entry(sig.clone()).or_default().push(dir.clone());
    }
    let mut groups = Vec::new();
    for members in by_sig.into_values() {
        if members.len() < 2 {
            continue;
        }
        let mut filtered: Vec<PathBuf> = Vec::new();
        for member in members {
            if filtered
                .iter()
                .any(|m| member.starts_with(m) || m.starts_with(&member))
            {
                continue;
            }
            filtered.push(member);
        }
        if filtered.len() < 2 {
            continue;
        }
        let bytes = file_counts.get(&filtered[0]).copied().unwrap_or(0);
        let total_bytes = byte_counts.get(&filtered[0]).copied().unwrap_or(0);
        let sizes = vec![total_bytes; filtered.len()];
        groups.push(FindingGroup {
            label: format!("Identical folder tree ({} copies)", filtered.len()),
            detail: format!(
                "{bytes} {}, {}",
                if bytes == 1 { "file" } else { "files" },
                crate::util::human_bytes(total_bytes)
            ),
            reclaimable: total_bytes.saturating_mul(filtered.len() as u64 - 1),
            members: filtered,
            sizes,
            bytes: total_bytes,
        });
    }
    groups.sort_by_key(|g| std::cmp::Reverse(g.reclaimable));
    (groups, notes)
}
