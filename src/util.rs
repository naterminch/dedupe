use anyhow::{Result, bail};
use std::process::Command;

/// Spawn a helper process without ever flashing a console window.
///
/// On Windows, child processes of a GUI-subsystem app (dedupe-gui) get
/// their own console window by default — every ffmpeg/ffprobe probe would
/// blink a terminal. `CREATE_NO_WINDOW` suppresses that. Children also run
/// at below-normal priority so a big media scan stays out of the way of
/// everything else on the machine. All our children have piped output, so
/// this is also harmless for the CLI binary.
pub fn quiet_command(program: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS
        let mut cmd = Command::new(program);
        cmd.creation_flags(0x0800_0000 | 0x0000_4000);
        cmd
    }
    #[cfg(not(windows))]
    {
        Command::new(program)
    }
}

/// Resolve a `--jobs` value to a concrete worker count: an explicit value
/// wins, otherwise use all but one core (at least 1) so the machine stays
/// responsive while a scan saturates the rest.
pub fn effective_jobs(requested: usize) -> usize {
    if requested > 0 {
        return requested;
    }
    std::thread::available_parallelism()
        .map(|n| n.get().saturating_sub(1).max(1))
        .unwrap_or(4)
}

/// Size rayon's global thread pool for this run. Safe to call on every
/// scan: only the first call takes effect, later ones are ignored (rayon
/// forbids re-initialization, which is fine — the pool already exists).
/// Tests use rayon's default pool; only the real pipeline calls this.
pub fn init_thread_pool(jobs: usize) {
    let threads = effective_jobs(jobs);
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("dedupe-{i}"))
        .build_global();
}

/// Format a byte count as a human-readable string (decimal units).
pub fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.2} {}", UNITS[unit])
    }
}

/// Format milliseconds as `H:MM:SS` (or `M:SS` when under an hour).
pub fn format_duration_ms(ms: u64) -> String {
    let total_secs = ms / 1000;
    let secs = total_secs % 60;
    let mins = (total_secs / 60) % 60;
    let hours = total_secs / 3600;
    if hours > 0 {
        format!("{hours}:{mins:02}:{secs:02}")
    } else {
        format!("{mins}:{secs:02}")
    }
}

/// Parse a human size string like `512`, `1.5KB`, `2 MB`, `3GiB`, `4G`.
/// Decimal suffixes (KB, MB, ...) multiply by powers of 1000;
/// binary suffixes (KiB, MiB, ...) and bare letters (K, M, ...) by powers of 1024.
pub fn parse_size(raw: &str) -> Result<u64> {
    let s = raw.trim();
    if s.is_empty() {
        bail!("empty size");
    }
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num_part, suffix) = s.split_at(split);
    let value: f64 = num_part
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid size: '{raw}'"))?;
    let suffix = suffix.trim().to_ascii_uppercase();
    let multiplier: f64 = match suffix.as_str() {
        "" | "B" => 1.0,
        "KB" => 1_000.0_f64,
        "MB" => 1_000.0_f64.powi(2),
        "GB" => 1_000.0_f64.powi(3),
        "TB" => 1_000.0_f64.powi(4),
        "K" | "KIB" | "KI" => 1024.0_f64,
        "M" | "MIB" | "MI" => 1024.0_f64.powi(2),
        "G" | "GIB" | "GI" => 1024.0_f64.powi(3),
        "T" | "TIB" | "TI" => 1024.0_f64.powi(4),
        other => bail!("unknown size suffix '{other}' in '{raw}'"),
    };
    let bytes = (value * multiplier).round();
    if bytes < 0.0 || bytes > u64::MAX as f64 {
        bail!("size out of range: '{raw}'");
    }
    Ok(bytes as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_bytes_scales() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(999), "999 B");
        assert_eq!(human_bytes(1_500), "1.50 KB");
        assert_eq!(human_bytes(2_621_440), "2.62 MB");
    }

    #[test]
    fn parse_size_variants() {
        assert_eq!(parse_size("512").unwrap(), 512);
        assert_eq!(parse_size("1.5KB").unwrap(), 1_500);
        assert_eq!(parse_size("2 MB").unwrap(), 2_000_000);
        assert_eq!(parse_size("3GiB").unwrap(), 3 * 1024 * 1024 * 1024);
        assert_eq!(parse_size("4G").unwrap(), 4 * 1024 * 1024 * 1024);
        assert_eq!(parse_size("0").unwrap(), 0);
        assert!(parse_size("abc").is_err());
        assert!(parse_size("1XB").is_err());
    }

    #[test]
    fn format_duration() {
        assert_eq!(format_duration_ms(65_000), "1:05");
        assert_eq!(format_duration_ms(3_661_000), "1:01:01");
    }

    #[test]
    fn effective_jobs_honors_explicit_and_stays_sane_on_auto() {
        assert_eq!(effective_jobs(1), 1);
        assert_eq!(effective_jobs(4), 4);
        // Auto (0) leaves headroom but never drops to zero workers.
        assert!(effective_jobs(0) >= 1);
    }

    #[test]
    fn init_thread_pool_is_safe_to_call_repeatedly() {
        // Only the first call takes effect; later ones are ignored, never
        // panics — the pipeline calls this on every scan.
        init_thread_pool(1);
        init_thread_pool(2);
        init_thread_pool(0);
    }

    #[test]
    fn quiet_command_targets_the_requested_program() {
        assert_eq!(quiet_command("ffmpeg").get_program(), "ffmpeg");
        assert_eq!(quiet_command("ffprobe").get_program(), "ffprobe");
    }

    #[test]
    fn human_bytes_covers_zero_and_terabytes() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1_500_000_000_000), "1.50 TB");
    }
}
