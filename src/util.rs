use anyhow::{Result, bail};
use std::process::Command;

/// Spawn a helper process without ever flashing a console window.
///
/// On Windows, ffmpeg/ffprobe are console programs: when a search runs
/// from the GUI, each spawn would otherwise blink its own terminal.
/// `CREATE_NO_WINDOW` suppresses that. Children also run
/// at below-normal priority so a big media scan stays out of the way of
/// everything else on the machine. All our children have piped output, so
/// this is also harmless for CLI scans, whose children attach to the
/// existing console.
#[must_use]
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

/// Tie every helper process to this one's lifetime (Windows).
///
/// `ffmpeg` and `ffprobe` are spawned for poster frames, media probes and
/// video fingerprints. On Windows a child outlives its parent by default, so
/// closing the window mid-extraction — or ending the app from Task Manager —
/// would leave decoders running with no window left to report to. Putting the
/// process in a job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` makes
/// Windows terminate the whole tree with it: the job handle stays open for
/// the life of the process, so it closes exactly when the process ends.
///
/// Call this first thing in `main`, before anything spawns: a child created
/// earlier is not in the job. Best effort — if the process already belongs to
/// a job that forbids nesting, children outlive the app as before.
#[cfg(windows)]
pub fn own_child_processes() {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    // SAFETY: the job handle is ours, `info` outlives the call and the byte
    // count matches its type; GetCurrentProcess returns a pseudo-handle that
    // needs no closing.
    unsafe {
        let Ok(job) = CreateJobObjectW(None, None) else {
            return;
        };
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            std::ptr::addr_of!(info).cast(),
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
        .is_ok();
        let assigned = AssignProcessToJobObject(job, GetCurrentProcess()).is_ok();
        if !configured || !assigned {
            let _ = CloseHandle(job);
        }
        // On success the handle is deliberately never closed: closing it is
        // what kills the children, and we want that at process exit, not now.
    }
}

/// No-op counterpart: on macOS and Linux the helper processes are short-lived
/// ffmpeg/ffprobe runs, and a GUI launch is normally a single window the user
/// closes only after the app has finished with them.
#[cfg(not(windows))]
pub fn own_child_processes() {}

/// Whether a standard stream was redirected to a file or pipe.
///
/// Redirected streams must survive console attachment untouched: `dedupe
/// ... > report.txt`, `| jq` and the integration tests all capture output
/// through a disk or pipe handle, and replacing it with `CONOUT$` would
/// throw that redirection away.
#[cfg(windows)]
fn stream_is_redirected(std: windows::Win32::System::Console::STD_HANDLE) -> bool {
    use windows::Win32::Storage::FileSystem::{FILE_TYPE_DISK, FILE_TYPE_PIPE, GetFileType};
    use windows::Win32::System::Console::GetStdHandle;

    // SAFETY: both calls take or return plainly-initialized handles and
    // dereference no caller memory.
    match unsafe { GetStdHandle(std) } {
        Ok(handle) if !handle.is_invalid() => {
            // SAFETY: `handle` came from GetStdHandle and is not invalid;
            // GetFileType only inspects the handle it is given.
            let kind = unsafe { GetFileType(handle) };
            kind == FILE_TYPE_DISK || kind == FILE_TYPE_PIPE
        }
        _ => false,
    }
}

/// Point the standard streams at the terminal that launched this process.
///
/// The binary is linked as a GUI-subsystem app on Windows (`windows_subsystem
/// = "windows"`), so Windows never creates a console for it: opening the
/// desktop app from Explorer, the Run dialog or a shortcut shows no terminal
/// at all — not even the brief flash that hiding an already-created console
/// leaves behind. The price is that a CLI run started from cmd, PowerShell or
/// Windows Terminal also gets no console, so its reports, progress bars and
/// `y/n/a/q` prompts would go nowhere.
///
/// Attaching to the parent console and reopening `CONIN$`/`CONOUT$` restores
/// normal console behavior for those runs. Streams that are already redirected
/// to a file or pipe are left alone, so captured output keeps working.
#[cfg(windows)]
pub fn attach_parent_console() {
    use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Console::{
        ATTACH_PARENT_PROCESS, AttachConsole, GetConsoleWindow, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
        STD_OUTPUT_HANDLE, SetStdHandle,
    };
    use windows::core::w;

    // SAFETY: AttachConsole takes no pointers; failing only means there is no
    // parent console to attach to (Explorer, a service, CI without a console).
    let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_ok();
    // SAFETY: GetConsoleWindow returns this process's console HWND or null.
    let has_console = !unsafe { GetConsoleWindow() }.0.is_null();
    if !attached && !has_console {
        return;
    }

    for (std, device, access) in [
        (STD_INPUT_HANDLE, w!("CONIN$"), GENERIC_READ),
        (STD_OUTPUT_HANDLE, w!("CONOUT$"), GENERIC_WRITE),
        (STD_ERROR_HANDLE, w!("CONOUT$"), GENERIC_WRITE),
    ] {
        if stream_is_redirected(std) {
            continue;
        }
        // SAFETY: `device` is a static null-terminated wide string, the
        // security-attributes and template handles are optional, and
        // OPEN_EXISTING only needs the console device to be present.
        let Ok(handle) = (unsafe {
            CreateFileW(
                device,
                access.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                Default::default(),
                None,
            )
        }) else {
            continue;
        };
        // SAFETY: `handle` was just opened on the console we are attached to.
        unsafe { SetStdHandle(std, handle).ok() };
    }
}

/// No-op counterpart: on macOS and Linux a GUI launch from a file manager or
/// the desktop already gets no terminal, and one from a shell keeps the user's
/// own window, whose output must keep working for CLI runs.
#[cfg(not(windows))]
pub fn attach_parent_console() {}

/// Resolve a `--jobs` value to a concrete worker count: an explicit value
/// wins, otherwise use all but one core (at least 1) so the machine stays
/// responsive while a scan saturates the rest.
#[must_use]
pub fn effective_jobs(requested: usize) -> usize {
    if requested > 0 {
        return requested;
    }
    std::thread::available_parallelism().map_or(4, |n| n.get().saturating_sub(1).max(1))
}

/// Size rayon's global thread pool for this run.
///
/// Safe to call on every scan: only the first call takes effect, later ones
/// are ignored (rayon forbids re-initialization, which is fine - the pool
/// already exists). Tests use rayon's default pool; only the real pipeline
/// calls this.
pub fn init_thread_pool(jobs: usize) {
    let threads = effective_jobs(jobs);
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("dedupe-{i}"))
        // Scan workers give way to the app's own threads: while a big folder
        // hashes, the window, its renderer and the input pump keep their
        // slices, so the GUI answers instead of going sluggish mid-scan.
        .start_handler(|_| lower_thread_priority())
        .build_global();
}

/// Run the calling thread below normal priority.
///
/// Best effort: a platform that refuses the change keeps the default.
#[cfg(windows)]
fn lower_thread_priority() {
    use windows::Win32::System::Threading::{
        GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
    };
    // SAFETY: both calls act on the calling thread and take no pointers; a
    // failure only leaves the worker at its default priority.
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

/// No thread priority to lower off Windows.
#[cfg(not(windows))]
fn lower_thread_priority() {}

/// Format a byte count as a human-readable string (decimal units).
#[must_use]
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
#[must_use]
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
///
/// Decimal suffixes (KB, MB, ...) multiply by powers of 1000;
/// binary suffixes (KiB, MiB, ...) and bare letters (K, M, ...) by powers of 1024.
///
/// # Errors
///
/// Returns an error when the string is empty, the numeric part does not parse,
/// the suffix is unknown, or the result does not fit in a `u64`.
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
