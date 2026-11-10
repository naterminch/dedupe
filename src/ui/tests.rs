//! Headless tests of the desktop shell: the real app on an nsh engine
//! without a window, plus the state transitions behind its controls.
//!
//! The engine tests drive the production [`DedupeApp`] root through the
//! steps a user takes (press, type, submit) and assert on what the screen
//! shows. Prefs are pointed at a temp file under [`ENV_LOCK`] (the env var
//! is process-global), so nothing here touches the real
//! `~/.dedupe/dedupe.json`.

use std::path::PathBuf;
use std::sync::{MutexGuard, PoisonError};

use nsh_core::Size;
use nsh_core::element::Root;
use nsh_core::prelude::px;
use nsh_engine::EngineConfig;
use nsh_testing::Headless;

use crate::matching::{Group, GroupMember};
use crate::media::MediaKind;
use crate::prefs::ENV_LOCK;
use crate::ui::actions;
use crate::ui::root::DedupeApp;
use crate::ui::state::AppState;

/// One headless engine on a temp prefs file, holding [`ENV_LOCK`] for the
/// whole test (bootstrap loads prefs on mount).
fn start() -> (Headless, PrefsGuard) {
    let lock = ENV_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
    let path = std::env::temp_dir().join(format!(
        "dedupe-ui-prefs-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // SAFETY: ENV_LOCK is held for the whole test, so no other test thread
    // reads or writes the environment meanwhile.
    unsafe { std::env::set_var("DEDUPE_PREFS", &path) };
    let config = EngineConfig {
        viewport: Size::new(px(1120.0), px(640.0)),
        ..EngineConfig::new(Root::of::<DedupeApp>())
    };
    let app = Headless::start(config).expect("the engine starts");
    (app, PrefsGuard { _lock: lock, path })
}

struct PrefsGuard {
    _lock: MutexGuard<'static, ()>,
    path: PathBuf,
}

impl Drop for PrefsGuard {
    fn drop(&mut self) {
        // SAFETY: `_lock` drops after this, so the lock is still held.
        unsafe { std::env::remove_var("DEDUPE_PREFS") };
        let _ = std::fs::remove_file(&self.path);
    }
}

fn member(path: &str, size: u64, keep: bool, similarity: Option<f64>) -> GroupMember {
    GroupMember {
        path: PathBuf::from(path),
        size,
        mtime_secs: None,
        keep,
        reference: false,
        media: None,
        similarity,
        content_hash: None,
        fingerprint_res: None,
    }
}

fn group(index: usize, members: Vec<GroupMember>) -> Group {
    Group {
        index,
        hash: format!("test-hash-{index}"),
        media_kind: MediaKind::Other,
        similarity: None,
        members,
    }
}

#[test]
fn shell_shows_the_core_controls() {
    let (mut app, _guard) = start();
    app.wait_for("text:dedupe").expect("header title");
    app.wait_for("text:Scan for duplicates")
        .expect("scan button");
    assert!(app.shows("Settings"), "settings button");
    assert!(app.shows("Source folders"), "sidebar heading");
}

#[test]
fn settings_page_opens_and_closes() {
    let (mut app, _guard) = start();
    app.wait_for("text:Settings").expect("settings button");
    app.press("text:Settings").expect("open settings");
    app.wait_for("text:Back").expect("back button");
    app.wait_for("text:Preferences").expect("settings nav");
    app.press("text:Back").expect("close settings");
    app.wait_for("text:Scan for duplicates")
        .expect("main flow returns");
}

#[test]
fn scan_without_folders_explains_what_is_missing() {
    let (mut app, _guard) = start();
    app.wait_for("text:Scan for duplicates")
        .expect("scan button");
    app.press("text:Scan for duplicates").expect("press scan");
    app.wait_for("text:/Add a folder/")
        .expect("status names the missing folder");
}

#[test]
fn adding_a_folder_updates_the_sidebar() {
    let (mut app, _guard) = start();
    app.wait_for("text:Folder path").expect("folder input");
    // Paste is one edit (a single change event): typing a path one key at a
    // time loses the keys after the first, because the placeholder's
    // removal shifts the node ids the harness cached for the input.
    let mut pasted = nsh_core::PasteEvent::default();
    pasted.text = r"C:\media".to_owned();
    app.paste("text:Folder path", &pasted)
        .expect("paste the path");
    app.press("text:Add").expect("press add");
    app.wait_for(r"text:C:\media").expect("folder row appears");
    app.wait_for("text:Remove").expect("row controls");
}

#[test]
fn select_all_duplicates_skips_keepers() {
    let mut s = AppState {
        groups: vec![group(
            1,
            vec![
                member("a", 10, true, None),
                member("b", 10, false, None),
                member("c", 10, false, None),
            ],
        )],
        ..AppState::default()
    };
    actions::reselect_all_dups(&mut s);
    assert_eq!(s.selected.len(), 2);
    assert!(s.selected.contains(&PathBuf::from("b")));
    assert!(!s.selected.contains(&PathBuf::from("a")));
}

#[test]
fn recount_counts_duplicates_and_reclaimable_bytes() {
    let mut s = AppState {
        groups: vec![
            group(
                1,
                vec![member("a", 100, true, None), member("b", 100, false, None)],
            ),
            group(
                2,
                vec![
                    member("c", 50, true, None),
                    member("d", 50, false, None),
                    member("e", 50, false, None),
                ],
            ),
        ],
        ..AppState::default()
    };
    actions::recount(&mut s);
    assert_eq!(s.dup_files, 3);
    assert_eq!(s.reclaim_bytes, 200);
}

#[test]
fn similarity_floor_drops_members_below_it() {
    let members = vec![
        member("keep.jpg", 300, true, None),
        member("close.jpg", 200, false, Some(0.985)),
        member("far.jpg", 100, false, Some(0.95)),
    ];
    let mut s = AppState {
        scan_similarity: 0.95,
        groups: vec![group(1, members.clone())],
        base_groups: vec![group(1, members)],
        ..AppState::default()
    };
    actions::apply_threshold(&mut s, 0.98);
    assert!((s.min_sim - 0.98).abs() < 1e-9);
    assert_eq!(s.groups[0].members.len(), 2);
    assert!(
        s.groups[0]
            .members
            .iter()
            .all(|m| !m.path.ends_with("far.jpg"))
    );
}

#[test]
fn biggest_first_ordering_puts_the_most_reclaimable_group_first() {
    let base = vec![
        group(
            1,
            vec![member("a", 10, true, None), member("b", 10, false, None)],
        ),
        group(
            2,
            vec![member("c", 90, true, None), member("d", 90, false, None)],
        ),
    ];
    let mut s = AppState {
        groups: base.clone(),
        base_groups: base,
        sort_biggest: true,
        ..AppState::default()
    };
    actions::sort_groups(&mut s);
    assert_eq!(s.groups[0].index, 2);
}

#[test]
fn pending_targets_are_selected_duplicates_only() {
    let s = AppState {
        groups: vec![group(
            1,
            vec![
                member("keep", 10, true, None),
                member("dup", 10, false, None),
            ],
        )],
        selected: [PathBuf::from("keep"), PathBuf::from("dup")]
            .into_iter()
            .collect(),
        ..AppState::default()
    };
    let targets = actions::pending_targets(&s);
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].path, PathBuf::from("dup"));
}

#[test]
fn scanning_without_folders_is_refused_with_a_helpful_message() {
    let s = AppState::default();
    let err = actions::collect_options(&s).expect_err("no folders to scan");
    assert!(err.contains("Add a folder"), "unexpected message: {err}");
}

#[test]
fn scan_reports_the_duplicate_groups_it_found() {
    let (mut app, _guard) = start();
    // A real folder with one duplicate pair and one unrelated file: the
    // pipeline runs for keeps (walk, hash, group), so this exercises the
    // async scan path end to end, not just state.
    let dir = std::env::temp_dir().join(format!(
        "dedupe-scan-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    for name in ["one.txt", "two.txt"] {
        std::fs::write(dir.join(name), vec![b'x'; 4096]).expect("fixture file");
    }
    std::fs::write(dir.join("unique.txt"), b"different bytes entirely").expect("unique file");
    // Paste is one edit (same reason as the folder-add test above).
    let result = (|| {
        app.wait_for("text:Folder path")?;
        let mut pasted = nsh_core::PasteEvent::default();
        pasted.text = dir.display().to_string();
        app.paste("text:Folder path", &pasted)?;
        app.press("text:Add")?;
        app.wait_for("text:Remove")?;
        app.press("text:Scan for duplicates")?;
        // The scan pumps on the wall clock (200 ms timer loop), so poll the
        // rendered summary for up to a minute of real time.
        app.wait_for_within("text:/group\\(s\\)/", std::time::Duration::from_secs(60))
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result.expect("scan produces the duplicate summary");
}

/// A scan that finds media must render its groups: every member row draws a
/// thumbnail (photo from the file itself, video from a poster frame), and a
/// video whose poster cannot be extracted must not keep the list from
/// rendering.
#[test]
fn scan_renders_media_groups_and_their_thumbnails() {
    let (mut app, _guard) = start();
    let dir = std::env::temp_dir().join(format!(
        "dedupe-media-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    let photo = dir.join("one.png");
    image::RgbImage::from_pixel(8, 8, image::Rgb([200, 30, 40]))
        .save(&photo)
        .expect("write fixture png");
    std::fs::copy(&photo, dir.join("two.png")).expect("copy fixture png");
    // Videos with identical bytes that no decoder accepts: the worst case
    // for the preview path (poster extraction fails).
    std::fs::write(dir.join("one.mp4"), vec![7u8; 4096]).expect("fixture video");
    std::fs::copy(dir.join("one.mp4"), dir.join("two.mp4")).expect("copy fixture video");
    let result = (|| {
        app.wait_for("text:Folder path")?;
        let mut pasted = nsh_core::PasteEvent::default();
        pasted.text = dir.display().to_string();
        app.paste("text:Folder path", &pasted)?;
        app.press("text:Add")?;
        app.wait_for("text:Remove")?;
        app.press("text:Scan for duplicates")?;
        app.wait_for_within("text:/group\\(s\\)/", std::time::Duration::from_secs(60))?;
        // The member rows themselves, not just the summary chip.
        app.wait_for("text:/one\\.png/")
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result.expect("media groups render with their rows");
}
