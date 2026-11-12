mod csv;
mod html;
mod human;
mod json;

use crate::matching::Group;
use crate::media::MediaKind;

pub use csv::write_csv;
pub use html::write_html;
pub use human::{print_human, print_mode_human};
pub use json::{print_json, print_mode_json, to_json, to_mode_json};

/// What every report format needs to know about the run itself.
#[derive(Debug)]
pub struct ReportContext<'a> {
    /// The scan roots as the user passed them, for the report header.
    pub paths: &'a [String],
    /// Algorithm name shown alongside hashes.
    pub hash_algo: &'a str,
    /// Include per-member detail that the compact forms leave out.
    pub verbose: bool,
    /// Whether media probing ran, so the report can say why metadata is
    /// missing rather than implying there was none.
    pub ffprobe_used: bool,
}

/// Aggregate counters captured from the scan phase (the entry list itself is
/// consumed by the hashing pipeline).
#[derive(Debug, Clone)]
pub struct ScanStats {
    /// Regular files the walker handed to hashing.
    pub files_scanned: usize,
    /// Total size of those files.
    pub bytes_scanned: u64,
    /// Directories skipped by depth, exclusion or ignore rules.
    pub dirs_skipped: u64,
    /// Files skipped by size, type or ignore rules.
    pub files_skipped: u64,
    /// Hardlink twins counted once, so their space is not reported twice.
    pub hardlinks_skipped: u64,
    /// Directories that could not be read, with the reason.
    pub dir_read_errors: Vec<String>,
}

/// Word a report uses for a group's media kind: `"image"`, `"video"` or
/// `"file"`.
#[must_use]
pub fn group_kind_name(kind: MediaKind) -> &'static str {
    match kind {
        MediaKind::Image => "image",
        MediaKind::Video => "video",
        MediaKind::Other => "file",
    }
}

/// How many files could be deleted without losing content: every group member
/// that is not a keeper.
#[must_use]
pub fn dup_file_count(groups: &[Group]) -> u64 {
    groups
        .iter()
        .map(|g| (g.members.len() - g.keep_count()) as u64)
        .sum()
}

/// Bytes freed by deleting every non-keeper member of every group.
pub fn reclaimable_bytes(groups: &[Group]) -> u64 {
    groups.iter().map(Group::dup_bytes).sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matching::GroupMember;

    #[test]
    fn to_json_has_expected_shape() {
        let paths = ["/media".to_string()];
        let ctx = ReportContext {
            paths: &paths,
            hash_algo: "blake3",
            verbose: false,
            ffprobe_used: false,
        };
        let stats = ScanStats {
            files_scanned: 2,
            bytes_scanned: 20,
            dirs_skipped: 0,
            files_skipped: 0,
            hardlinks_skipped: 1,
            dir_read_errors: Vec::new(),
        };
        let v: serde_json::Value =
            serde_json::from_str(&to_json(&ctx, &stats, &[])).expect("valid JSON");
        assert_eq!(v["hash_algorithm"], "blake3");
        assert_eq!(v["hardlinks_skipped"], 1);
        assert_eq!(v["duplicate_groups"], 0);
    }

    #[test]
    fn to_mode_json_has_expected_shape() {
        use crate::cli::FindMode;
        use crate::modes::{Finding, FindingGroup, ModeResult};
        use std::path::PathBuf;

        let paths = ["/media".to_string()];
        let ctx = ReportContext {
            paths: &paths,
            hash_algo: "blake3",
            verbose: false,
            ffprobe_used: false,
        };
        let result = ModeResult {
            mode: FindMode::EmptyFiles,
            findings: vec![Finding {
                path: PathBuf::from("a.txt"),
                size: 0,
                kind: "empty",
                detail: "zero-byte file".to_string(),
                is_dir: false,
                hash: None,
            }],
            groups: vec![FindingGroup {
                label: "Identical folder tree (2 copies)".to_string(),
                detail: "1 file, 6 B".to_string(),
                members: vec![PathBuf::from("one"), PathBuf::from("two")],
                sizes: vec![6, 6],
                hashes: vec![None, None],
                bytes: 6,
                reclaimable: 6,
            }],
            files_scanned: 2,
            bytes_scanned: 6,
            dirs_scanned: 1,
            files_skipped: 0,
            dirs_skipped: 0,
            errors: vec!["denied".to_string()],
            notes: vec!["note".to_string()],
            cancelled: false,
        };

        let v: serde_json::Value =
            serde_json::from_str(&to_mode_json(&ctx, &result)).expect("valid JSON");
        assert_eq!(v["mode"], "empty-files");
        assert_eq!(v["finding_count"], 1);
        assert_eq!(v["group_count"], 1);
        assert_eq!(v["reclaimable_bytes"], 6);
        assert_eq!(v["findings"][0]["kind"], "empty");
        assert_eq!(v["findings"][0]["size"], 0);
        assert_eq!(v["findings"][0]["is_dir"], false);
        assert_eq!(v["groups"][0]["members"].as_array().unwrap().len(), 2);
        assert_eq!(v["groups"][0]["reclaimable_bytes"], 6);
        assert_eq!(v["dir_read_errors"], serde_json::json!(["denied"]));
    }

    #[test]
    fn csv_and_html_exports_cover_modes_and_duplicates() {
        use crate::cli::FindMode;
        use crate::modes::{Finding, ModeResult};
        use std::path::PathBuf;

        let root = std::env::temp_dir().join(format!(
            "dedupe-report-export-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let csv_path = root.join("mode.csv");
        let html_path = root.join("mode.html");
        let dup_csv_path = root.join("duplicates.csv");
        let dup_html_path = root.join("duplicates.html");

        let paths = ["/media".to_string()];
        let ctx = ReportContext {
            paths: &paths,
            hash_algo: "blake3",
            verbose: false,
            ffprobe_used: false,
        };
        let stats = ScanStats {
            files_scanned: 3,
            bytes_scanned: 76,
            dirs_skipped: 0,
            files_skipped: 0,
            hardlinks_skipped: 0,
            dir_read_errors: Vec::new(),
        };
        let mode_result = ModeResult {
            mode: FindMode::BigFiles,
            findings: vec![Finding {
                path: PathBuf::from("a.bin"),
                size: 12,
                kind: "big",
                detail: "#1 of 3 scanned files".to_string(),
                is_dir: false,
                hash: None,
            }],
            files_scanned: 3,
            bytes_scanned: 76,
            ..ModeResult::default()
        };
        write_csv(&csv_path, &ctx, &stats, &[], Some(&mode_result)).unwrap();
        write_html(&html_path, &ctx, &stats, &[], Some(&mode_result)).unwrap();
        let csv = std::fs::read_to_string(&csv_path).unwrap();
        assert!(csv.starts_with("mode,kind,group,path,size,detail\n"));
        assert!(csv.contains("big-files,big,,a.bin,12,#1 of 3 scanned files"));
        let html = std::fs::read_to_string(&html_path).unwrap();
        assert!(html.contains("<h1>Largest files</h1>"));
        assert!(html.contains("<h2>Findings</h2>"));
        assert!(html.contains("a.bin"));

        let groups = vec![group(
            1,
            MediaKind::Other,
            vec![member("k.txt", 100, true), member("d.txt", 100, false)],
        )];
        write_csv(&dup_csv_path, &ctx, &stats, &groups, None).unwrap();
        write_html(&dup_html_path, &ctx, &stats, &groups, None).unwrap();
        let csv = std::fs::read_to_string(&dup_csv_path).unwrap();
        assert!(csv.contains("duplicates,keep,#1,k.txt,100,abc123"));
        assert!(csv.contains("duplicates,duplicate,#1,d.txt,100,abc123"));
        let html = std::fs::read_to_string(&dup_html_path).unwrap();
        assert!(html.contains("<h1>Duplicate files</h1>"));
        assert!(html.contains("Group #1"));
        assert!(html.contains("k.txt"));

        let _ = std::fs::remove_dir_all(&root);
    }

    fn member(path: &str, size: u64, keep: bool) -> GroupMember {
        GroupMember {
            path: std::path::PathBuf::from(path),
            size,
            mtime_secs: None,
            keep,
            reference: false,
            media: None,
            similarity: None,
            content_hash: None,
            fingerprint_res: None,
        }
    }

    fn group(index: usize, kind: MediaKind, members: Vec<GroupMember>) -> Group {
        Group {
            index,
            hash: "abc123".to_string(),
            media_kind: kind,
            similarity: None,
            members,
        }
    }

    #[test]
    fn group_kind_names_cover_every_media_kind() {
        assert_eq!(group_kind_name(MediaKind::Image), "image");
        assert_eq!(group_kind_name(MediaKind::Video), "video");
        assert_eq!(group_kind_name(MediaKind::Other), "file");
    }

    #[test]
    fn counters_count_only_non_keep_members() {
        let groups = vec![
            group(
                1,
                MediaKind::Other,
                vec![
                    member("k.txt", 100, true),
                    member("d1.txt", 100, false),
                    member("d2.txt", 100, false),
                ],
            ),
            group(
                2,
                MediaKind::Image,
                vec![member("k.png", 50, true), member("d.png", 50, false)],
            ),
        ];
        assert_eq!(dup_file_count(&groups), 3);
        assert_eq!(reclaimable_bytes(&groups), 250);
        assert_eq!(dup_file_count(&[]), 0);
        assert_eq!(reclaimable_bytes(&[]), 0);
    }

    #[test]
    fn to_json_reports_group_math_and_members() {
        let paths = ["C:\\pics".to_string()];
        let ctx = ReportContext {
            paths: &paths,
            hash_algo: "sha256",
            verbose: false,
            ffprobe_used: true,
        };
        let stats = ScanStats {
            files_scanned: 3,
            bytes_scanned: 300,
            dirs_skipped: 1,
            files_skipped: 2,
            hardlinks_skipped: 0,
            dir_read_errors: vec!["denied".to_string()],
        };
        let groups = vec![group(
            1,
            MediaKind::Other,
            vec![member("k.txt", 100, true), member("d.txt", 100, false)],
        )];
        let v: serde_json::Value =
            serde_json::from_str(&to_json(&ctx, &stats, &groups)).expect("valid JSON");
        assert_eq!(v["duplicate_groups"], 1);
        assert_eq!(v["duplicate_files"], 1);
        assert_eq!(v["reclaimable_bytes"], 100);
        assert_eq!(v["dirs_skipped"], 1);
        assert_eq!(v["dir_read_errors"], serde_json::json!(["denied"]));
        assert_eq!(v["groups"][0]["members"].as_array().unwrap().len(), 2);
        // Per-group preview: kind keeps its renamed key, reclaimable is live.
        assert_eq!(v["groups"][0]["kind"], "other");
        assert_eq!(v["groups"][0]["reclaimable_bytes"], 100);
    }

    #[test]
    fn print_human_renders_skips_errors_and_similar_groups() {
        use crate::media::MediaInfo;
        let paths = ["C:\\pics".to_string()];
        let stats = ScanStats {
            files_scanned: 4,
            bytes_scanned: 400,
            dirs_skipped: 1,
            files_skipped: 2,
            hardlinks_skipped: 3,
            dir_read_errors: vec!["denied".to_string()],
        };
        let mut keeper = member("k.png", 100, true);
        keeper.media = Some(MediaInfo {
            width: Some(800),
            height: Some(600),
            duration_ms: None,
            codec: Some("png".to_string()),
        });
        let mut dup = member("d.png", 100, false);
        dup.similarity = Some(0.981);
        dup.media = Some(MediaInfo {
            width: Some(800),
            height: Some(600),
            duration_ms: None,
            codec: None,
        });
        let groups = vec![Group {
            index: 1,
            hash: "img".to_string(),
            media_kind: MediaKind::Image,
            similarity: Some(0.981),
            members: vec![keeper, dup],
        }];
        // Verbose exercises the per-member media/codec line.
        for verbose in [false, true] {
            let ctx = ReportContext {
                paths: &paths,
                hash_algo: "blake3",
                verbose,
                ffprobe_used: false,
            };
            // Must not panic on any branch; output goes to stdout.
            print_human(&ctx, &stats, &groups);
        }
        // Empty groups print the no-duplicates marker instead of dividers.
        let ctx = ReportContext {
            paths: &paths,
            hash_algo: "blake3",
            verbose: false,
            ffprobe_used: true,
        };
        print_human(&ctx, &stats, &[]);
    }
}
