use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn tmpdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "dedupe-e2e-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> (std::process::Output, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_dedupe"))
        .args(args)
        .output()
        .expect("failed to run dedupe binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    (out, stdout)
}

fn run_with_env(args: &[&str], envs: &[(&str, &str)]) -> (std::process::Output, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_dedupe"));
    cmd.args(args);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to run dedupe binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    (out, stdout)
}

#[test]
fn finds_duplicates_recursively_including_subfolders() {
    let dir = tmpdir("recursive");
    fs::create_dir_all(dir.join("sub").join("deeper")).unwrap();
    fs::write(dir.join("a.txt"), b"payload").unwrap();
    fs::write(dir.join("sub").join("b.txt"), b"payload").unwrap();
    fs::write(dir.join("sub").join("deeper").join("c.txt"), b"payload").unwrap();
    fs::write(dir.join("unique.txt"), b"totally unique").unwrap();

    let (out, stdout) = run(&["--json", dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    assert_eq!(parsed["duplicate_groups"], 1);
    assert_eq!(parsed["duplicate_files"], 2);
    assert_eq!(parsed["groups"][0]["members"].as_array().unwrap().len(), 3);
    assert!(out.status.success());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn types_flag_limits_scan_to_given_extensions() {
    let dir = tmpdir("types");
    fs::write(dir.join("a.jpg"), b"same").unwrap();
    fs::write(dir.join("b.jpg"), b"same").unwrap();
    fs::write(dir.join("a.png"), b"same").unwrap();
    fs::write(dir.join("b.png"), b"same").unwrap();

    let (_, stdout) = run(&["--json", "--types", "jpg", dir.to_str().unwrap()]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["duplicate_groups"], 1);
    let group = &parsed["groups"][0];
    let members = group["members"].as_array().unwrap();
    assert!(members.iter().all(|m| {
        m["path"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase()
            .ends_with(".jpg")
    }));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn delete_with_yes_removes_duplicates_keeps_one() {
    // Exact duplicates always share the same size, but --keep-smaller still
    // deterministically designates the keeper (unit-tested in matching.rs).
    let dir = tmpdir("deletion");
    let alpha = dir.join("alpha.txt");
    let beta = dir.join("beta.txt");
    fs::write(&alpha, b"same payload").unwrap();
    fs::write(&beta, b"same payload").unwrap();

    let (out, _) = run(&[
        "--delete",
        "--yes",
        "--keep-smaller",
        "--quiet",
        dir.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Exactly one copy survives.
    assert!(alpha.exists() ^ beta.exists());
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn no_duplicates_yields_empty_result() {
    let dir = tmpdir("clean");
    fs::write(dir.join("a.txt"), b"alpha").unwrap();
    fs::write(dir.join("b.txt"), b"beta").unwrap();

    let (_, stdout) = run(&["--json", dir.to_str().unwrap()]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["duplicate_groups"], 0);
    assert_eq!(parsed["groups"].as_array().unwrap().len(), 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn human_output_shows_styled_group_report() {
    let dir = tmpdir("human");
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("a.txt"), b"payload").unwrap();
    fs::write(dir.join("sub").join("b.txt"), b"payload").unwrap();
    fs::write(dir.join("unique.txt"), b"totally unique").unwrap();

    let (out, stdout) = run(&[dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Styled report markers (glyphs survive even when piped; only ANSI codes
    // are suppressed, and JSON output is never styled).
    assert!(stdout.contains("◆ Group #1"), "stdout: {stdout}");
    assert!(stdout.contains("✓ KEEP"), "stdout: {stdout}");
    assert!(stdout.contains("✗ DUP"), "stdout: {stdout}");
    assert!(stdout.contains("Tip:"), "stdout: {stdout}");
    // No ANSI escape codes when output is piped.
    assert!(
        !stdout.contains('\u{1b}'),
        "ANSI escapes leaked into piped stdout: {stdout:?}"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn similar_mode_finds_same_image_in_different_formats() {
    use image::ImageBuffer;

    // The same synthetic photo saved as PNG and (lossy) JPEG — byte-identical
    // hashes will NOT match, but the perceptual dHash should be >= 97%.
    let dir = tmpdir("similar");
    let png_path = dir.join("photo.png");
    let jpg_path = dir.join("photo.jpg");
    let img: ImageBuffer<image::Rgb<u8>, Vec<u8>> = ImageBuffer::from_fn(160, 120, |x, y| {
        let c = ((x * 2 + y * 3) % 256) as u8;
        image::Rgb([c, c.wrapping_mul(2), 255 - c])
    });
    img.save(&png_path).unwrap();
    img.save(&jpg_path).unwrap();

    let (out, stdout) = run(&["--json", dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(
        parsed["duplicate_groups"], 1,
        "expected one similar group, stdout: {stdout}"
    );
    let group = &parsed["groups"][0];
    assert_eq!(group["members"].as_array().unwrap().len(), 2);
    assert!(
        group["similarity"].as_f64().unwrap() >= 0.97,
        "group similarity below threshold: {group}"
    );
    // Exactly one member is designated the keeper.
    let keep_count = group["members"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["keep"].as_bool().unwrap())
        .count();
    assert_eq!(keep_count, 1);

    // With --exact the two formats are NOT duplicates (different bytes).
    let (_, plain) = run(&["--exact", "--json", dir.to_str().unwrap()]);
    let plain: serde_json::Value = serde_json::from_str(&plain).unwrap();
    assert_eq!(plain["duplicate_groups"], 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn similar_mode_finds_reencoded_video_in_different_container() {
    use std::process::Command as Proc;

    // Requires real ffmpeg + ffprobe to generate and fingerprint media; skip
    // cleanly on machines without them (like the image test, which runs
    // natively, the video pipeline is only exercised when the tools exist).
    fn tool(name: &str) -> bool {
        Proc::new(name)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    if !tool("ffmpeg") || !tool("ffprobe") {
        eprintln!("skipping: ffmpeg/ffprobe not on PATH");
        return;
    }

    let dir = tmpdir("similar-video");
    let mp4 = dir.join("clip.mp4");
    let mov = dir.join("clip.mov");

    // Deterministic synthetic source, then a re-encode into a different
    // container: same content, different bytes.
    let src = Proc::new("ffmpeg")
        .args([
            "-y",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=duration=2:size=320x240:rate=10",
        ])
        .args(["-pix_fmt", "yuv420p", mp4.to_str().unwrap()])
        .status()
        .expect("ffmpeg source encode failed");
    assert!(src.success());
    let re = Proc::new("ffmpeg")
        .args(["-y", "-loglevel", "error", "-i", mp4.to_str().unwrap()])
        .args([
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            mov.to_str().unwrap(),
        ])
        .status()
        .expect("ffmpeg re-encode failed");
    assert!(re.success(), "re-encode to .mov failed");

    let (out, stdout) = run(&["--json", dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");

    assert_eq!(
        parsed["duplicate_groups"], 1,
        "expected one similar video group, stdout: {stdout}"
    );
    let group = &parsed["groups"][0];
    assert_eq!(group["kind"], "video");
    assert_eq!(group["members"].as_array().unwrap().len(), 2);
    assert!(
        group["similarity"].as_f64().unwrap() >= 0.97,
        "re-encoded video similarity below threshold: {group}"
    );
    let keep_count = group["members"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["keep"].as_bool().unwrap())
        .count();
    assert_eq!(keep_count, 1);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn fingerprint_cache_is_persisted_and_reused_across_runs() {
    use std::process::Command as Proc;

    fn tool(name: &str) -> bool {
        Proc::new(name)
            .arg("-version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    if !tool("ffmpeg") || !tool("ffprobe") {
        eprintln!("skipping: ffmpeg/ffprobe not on PATH");
        return;
    }

    let dir = tmpdir("cache");
    // Keep the cache file OUTSIDE the scanned directory so it cannot change
    // the file count between runs.
    let cache_file = std::env::temp_dir().join(format!(
        "dedupe-e2e-cache-{}-{}.bin",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));

    // Two same-resolution, same-duration clips with different content: both
    // land in the same resolution bucket and get fingerprinted (they are not
    // byte-identical, so they reach the similar pass).
    for (name, src) in [
        ("clip1.mp4", "testsrc=duration=2:size=320x240:rate=10"),
        ("clip2.mp4", "testsrc2=duration=2:size=320x240:rate=10"),
    ] {
        let out = Proc::new("ffmpeg")
            .args(["-y", "-loglevel", "error", "-f", "lavfi", "-i", src])
            .args(["-pix_fmt", "yuv420p", dir.join(name).to_str().unwrap()])
            .status()
            .expect("ffmpeg encode failed");
        assert!(out.success());
    }

    let envs = [("DEDUPE_CACHE", cache_file.to_str().unwrap())];

    // Run 1: populates the cache with two video fingerprints.
    let (out1, stdout1) = run_with_env(&["--json", dir.to_str().unwrap()], &envs);
    assert!(
        out1.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out1.stderr)
    );
    let first: serde_json::Value = serde_json::from_str(&stdout1).expect("valid JSON");
    assert_eq!(
        first["duplicate_groups"], 0,
        "different clips must not be reported as duplicates"
    );
    let bytes = fs::read(&cache_file).expect("cache file created on first run");
    assert!(
        bytes.len() > 200,
        "cache should hold two video fingerprints, got {} bytes",
        bytes.len()
    );

    // Run 2: unchanged files are served from the cache; results identical.
    let (out2, stdout2) = run_with_env(&["--json", dir.to_str().unwrap()], &envs);
    assert!(
        out2.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out2.stderr)
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stdout2).unwrap(),
        first,
        "cached run must produce the same report"
    );

    // --no-cache ignores the cache and leaves it untouched.
    let before = fs::read(&cache_file).unwrap();
    let (out3, _) = run_with_env(&["--json", "--no-cache", dir.to_str().unwrap()], &envs);
    assert!(out3.status.success());
    assert_eq!(
        fs::read(&cache_file).unwrap(),
        before,
        "--no-cache must not rewrite the cache"
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn help_flag_prints_usage_and_succeeds() {
    // NOTE: a bare `dedupe` invocation now opens the GUI (needs a display),
    // so it is intentionally not exercised here.
    // `--help` still exits 0 and prints to stdout.
    let help = Command::new(env!("CARGO_BIN_EXE_dedupe"))
        .arg("--help")
        .output()
        .expect("failed to run dedupe binary");
    assert!(help.status.success());
    let help_stdout = String::from_utf8_lossy(&help.stdout);
    assert!(help_stdout.contains("Usage:"));

    // Unknown flags are still CLI usage errors (exit 2).
    let bad = Command::new(env!("CARGO_BIN_EXE_dedupe"))
        .arg("--no-such-flag")
        .output()
        .expect("failed to run dedupe binary");
    assert_eq!(
        bad.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&bad.stderr)
    );
}

#[test]
fn reference_dir_is_protected_from_deletion() {
    let dir = tmpdir("refdir");
    let incoming = dir.join("incoming");
    let backup = dir.join("backup");
    fs::create_dir_all(&incoming).unwrap();
    fs::create_dir_all(&backup).unwrap();
    // a.txt sorts first, but the backup copy must be kept instead.
    fs::write(incoming.join("a.txt"), b"payload").unwrap();
    fs::write(backup.join("z.txt"), b"payload").unwrap();

    let (out, stdout) = run(&[
        "--json",
        "--reference-dir",
        backup.to_str().unwrap(),
        dir.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let members = parsed["groups"][0]["members"].as_array().unwrap();
    assert_eq!(members.len(), 2);
    assert!(members[0]["keep"].as_bool().unwrap());
    assert!(members[0]["path"].as_str().unwrap().ends_with("z.txt"));

    // Deleting removes only the unprotected copy.
    let del = Command::new(env!("CARGO_BIN_EXE_dedupe"))
        .args([
            "--reference-dir",
            backup.to_str().unwrap(),
            "--delete",
            "--yes",
            dir.to_str().unwrap(),
        ])
        .output()
        .expect("failed to run dedupe binary");
    assert!(
        del.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&del.stderr)
    );
    assert!(backup.join("z.txt").exists(), "reference copy must survive");
    assert!(!incoming.join("a.txt").exists(), "unprotected copy deleted");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn dry_run_deletes_nothing() {
    let dir = tmpdir("dryrun");
    fs::write(dir.join("a.txt"), b"payload").unwrap();
    fs::write(dir.join("b.txt"), b"payload").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_dedupe"))
        .args(["--delete", "--yes", "--dry-run", dir.to_str().unwrap()])
        .output()
        .expect("failed to run dedupe binary");
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Dry run"), "stdout: {stdout}");
    assert!(dir.join("a.txt").exists());
    assert!(dir.join("b.txt").exists());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn keep_newest_prefers_recently_modified() {
    use std::time::{Duration, SystemTime};
    let dir = tmpdir("keepnew");
    fs::write(dir.join("old.txt"), b"payload").unwrap();
    fs::write(dir.join("new.txt"), b"payload").unwrap();
    let ft = fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(3600));
    fs::File::options()
        .write(true)
        .open(dir.join("old.txt"))
        .unwrap()
        .set_times(ft)
        .unwrap();

    let (out, stdout) = run(&["--json", "--keep-newest", dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let members = parsed["groups"][0]["members"].as_array().unwrap();
    assert!(members[0]["keep"].as_bool().unwrap());
    assert!(members[0]["path"].as_str().unwrap().ends_with("new.txt"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn keep_oldest_prefers_least_recently_modified() {
    use std::time::{Duration, SystemTime};
    let dir = tmpdir("keepold");
    fs::write(dir.join("old.txt"), b"payload").unwrap();
    fs::write(dir.join("new.txt"), b"payload").unwrap();
    let ft = fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(3600));
    fs::File::options()
        .write(true)
        .open(dir.join("old.txt"))
        .unwrap()
        .set_times(ft)
        .unwrap();

    let (out, stdout) = run(&["--json", "--keep-oldest", dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_str(&stdout).expect("valid JSON");
    let members = parsed["groups"][0]["members"].as_array().unwrap();
    assert!(members[0]["keep"].as_bool().unwrap());
    assert!(members[0]["path"].as_str().unwrap().ends_with("old.txt"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn min_and_max_size_bound_the_scan() {
    let dir = tmpdir("sizes");
    fs::write(dir.join("tiny-a.txt"), b"12345").unwrap();
    fs::write(dir.join("tiny-b.txt"), b"12345").unwrap();
    let big = vec![7u8; 200_000];
    fs::write(dir.join("big-a.bin"), &big).unwrap();
    fs::write(dir.join("big-b.bin"), &big).unwrap();

    // min-size hides the tiny pair, keeps the big one.
    let (_, stdout) = run(&["--json", "--min-size", "100KB", dir.to_str().unwrap()]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["duplicate_groups"], 1);
    assert!(
        parsed["groups"][0]["members"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with(".bin")
    );

    // max-size hides the big pair, keeps the tiny one.
    let (_, stdout) = run(&["--json", "--max-size", "1KB", dir.to_str().unwrap()]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["duplicate_groups"], 1);
    assert!(
        parsed["groups"][0]["members"][0]["path"]
            .as_str()
            .unwrap()
            .ends_with(".txt")
    );

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn exclude_dir_and_path_prune_matches() {
    let dir = tmpdir("excludes");
    fs::create_dir_all(dir.join("node_modules")).unwrap();
    fs::write(dir.join("a.txt"), b"payload").unwrap();
    fs::write(dir.join("node_modules").join("b.txt"), b"payload").unwrap();
    fs::write(dir.join("draft-final.txt"), b"payload").unwrap();

    // Baseline: all three are duplicates.
    let (_, stdout) = run(&["--json", dir.to_str().unwrap()]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["duplicate_groups"], 1);

    // Excluding the directory leaves a.txt + draft-final.txt.
    let (_, stdout) = run(&[
        "--json",
        "--exclude-dir",
        "node_modules",
        dir.to_str().unwrap(),
    ]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["duplicate_groups"], 1);
    assert_eq!(parsed["duplicate_files"], 1);

    // Excluding both leaves a single file: no groups.
    let (_, stdout) = run(&[
        "--json",
        "--exclude-dir",
        "node_modules",
        "--exclude-path",
        "draft",
        dir.to_str().unwrap(),
    ]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["duplicate_groups"], 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn max_depth_zero_stays_top_level() {
    let dir = tmpdir("depth");
    fs::create_dir_all(dir.join("sub")).unwrap();
    fs::write(dir.join("a.txt"), b"payload").unwrap();
    fs::write(dir.join("sub").join("b.txt"), b"payload").unwrap();

    let (_, stdout) = run(&["--json", "--max-depth", "0", dir.to_str().unwrap()]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["duplicate_groups"], 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn empty_directory_reports_no_duplicates() {
    let dir = tmpdir("emptydir");

    let (out, stdout) = run(&[dir.to_str().unwrap()]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("No duplicate"), "stdout: {stdout}");

    let (_, json) = run(&["--json", dir.to_str().unwrap()]);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["duplicate_groups"], 0);
    assert_eq!(parsed["files_scanned"], 0);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn invalid_options_fail_with_usage_error() {
    let dir = tmpdir("badflags");
    fs::write(dir.join("a.txt"), b"payload").unwrap();

    // Similarity outside 0–100 is rejected.
    let out = Command::new(env!("CARGO_BIN_EXE_dedupe"))
        .args(["--similarity", "101", dir.to_str().unwrap()])
        .output()
        .expect("failed to run dedupe binary");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--similarity"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Conflicting keep flags are rejected.
    let out = Command::new(env!("CARGO_BIN_EXE_dedupe"))
        .args(["--keep-smaller", "--keep-newest", dir.to_str().unwrap()])
        .output()
        .expect("failed to run dedupe binary");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("conflict"),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Unparsable size filter is rejected.
    let out = Command::new(env!("CARGO_BIN_EXE_dedupe"))
        .args(["--min-size", "huge-ish", dir.to_str().unwrap()])
        .output()
        .expect("failed to run dedupe binary");
    assert!(!out.status.success());

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn trash_delete_removes_files_from_their_location() {
    let dir = tmpdir("trashdel");
    fs::write(dir.join("a.txt"), b"payload").unwrap();
    fs::write(dir.join("b.txt"), b"payload").unwrap();

    let (out, _) = run(&[
        "--delete",
        "--yes",
        "--trash",
        "--quiet",
        dir.to_str().unwrap(),
    ]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn json_report_carries_provenance_fields() {
    let dir = tmpdir("provenance");
    fs::write(dir.join("a.txt"), b"payload").unwrap();
    fs::write(dir.join("b.txt"), b"payload").unwrap();

    let (_, stdout) = run(&["--json", dir.to_str().unwrap()]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["hash_algorithm"], "blake3");
    assert!(parsed.get("ffprobe_used").is_some());
    assert_eq!(parsed["files_scanned"], 2);
    assert!(parsed["bytes_scanned"].as_u64().unwrap() > 0);

    let (_, stdout) = run(&["--json", "--hash", "sha256", dir.to_str().unwrap()]);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["hash_algorithm"], "sha256");
    assert_eq!(parsed["duplicate_groups"], 1);

    let _ = fs::remove_dir_all(&dir);
}
