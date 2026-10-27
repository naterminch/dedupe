use crate::hashing::HashEngine;
use anyhow::Result;
use std::fs;
use std::path::Path;

pub(super) fn prompt(message: &str) -> String {
    use std::io::Write;
    print!("{message}");
    let _ = std::io::stdout().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    line
}

pub(super) fn label_expected(expected: &str) -> bool {
    expected.starts_with("size:") || expected.starts_with("name:") || expected.starts_with("fuzzy:")
}

pub(super) fn verify_expected(
    path: &Path,
    expected: &str,
    size: u64,
    engine: &HashEngine,
) -> Result<bool, String> {
    if label_expected(expected) {
        let meta = fs::metadata(path).map_err(|e| e.to_string())?;
        Ok(meta.is_file() && meta.len() == size)
    } else {
        let hash = engine.full(path).map_err(|e| e.to_string())?;
        Ok(hash == expected)
    }
}
