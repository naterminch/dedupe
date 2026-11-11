//! Headless tests of the desktop shell: the real app on an nsh engine
//! without a window, plus the state transitions behind its controls.
//!
//! The engine tests drive the production [`DedupeApp`] root through the
//! steps a user takes (press, type, submit) and assert on what the screen
//! shows. Prefs are pointed at a temp file under [`ENV_LOCK`] (the env var
//! is process-global), so nothing here touches the real
//! `~/.dedupe/dedupe.json`.

use std::path::{Path, PathBuf};
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
use crate::ui::state::{AppState, FolderEntry};

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
    assert!(
        app.screen().count("label:Settings").unwrap_or(0) > 0,
        "settings button"
    );
    assert!(app.shows("Folders"), "sidebar heading");
}

#[test]
fn settings_page_opens_and_closes() {
    let (mut app, _guard) = start();
    app.wait_for("label:Settings").expect("settings button");
    app.press("label:Settings").expect("open settings");
    app.wait_for("text:Preferences").expect("settings nav");
    app.press("label:Scan workspace").expect("close settings");
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
    app.wait_for("text:Scan a folder").expect("folder input");
    // Paste is one edit (a single change event): typing a path one key at a
    // time loses the keys after the first, because the placeholder's
    // removal shifts the node ids the harness cached for the input.
    let mut pasted = nsh_core::PasteEvent::default();
    pasted.text = r"C:\media".to_owned();
    app.paste("text:Scan a folder", &pasted)
        .expect("paste the path");
    app.press("label:Add folder").expect("press add");
    app.wait_for(r"text:C:\media").expect("folder row appears");
    app.wait_for("label:Remove folder").expect("row controls");
}

/// Reset puts every persisted setting back to its shipped default and leaves
/// the live session — here the folder list — untouched.
#[test]
fn resetting_settings_restores_the_defaults_and_keeps_the_session() {
    let mut s = AppState {
        types_input: "png".to_string(),
        min_size_input: "10MB".to_string(),
        exact: true,
        no_cache: true,
        trash: false,
        remember_folders: false,
        sort_biggest: true,
        find_index: Some(3),
        match_index: Some(2),
        image_hash_index: Some(4),
        hash_index: Some(2),
        keep_index: Some(3),
        partial_clips: true,
        music_by_content: true,
        folders: vec![FolderEntry {
            path: r"C:\media".to_string(),
            reference: true,
        }],
        ..AppState::default()
    };
    s.apply_prefs(&crate::prefs::GuiPrefs::default());
    assert_eq!(s.types_input, "");
    assert_eq!(s.min_size_input, crate::ui_helpers::NO_LIMIT_LABEL);
    assert!(s.trash, "trash is the default");
    assert!(s.remember_folders, "remembering folders is the default");
    assert_eq!(s.find_index, Some(0));
    assert_eq!(s.match_index, Some(0));
    assert_eq!(s.image_hash_index, Some(0));
    assert_eq!(s.hash_index, Some(0));
    assert_eq!(s.keep_index, Some(0));
    assert!(!s.exact);
    assert!(!s.no_cache);
    assert!(!s.partial_clips);
    assert!(!s.music_by_content);
    assert!(!s.sort_biggest);
    assert_eq!(s.folders.len(), 1, "the live folder list survives a reset");
    assert!(s.folders[0].reference);
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

/// A keeper pick is a per-group decision: it moves the highlight, keeps the
/// member order, and leaves the other groups' ticks alone.
#[test]
fn keeper_picks_move_only_the_highlight() {
    let mut s = AppState {
        groups: vec![
            group(
                1,
                vec![
                    member("a", 10, true, None),
                    member("b", 10, false, None),
                    member("c", 10, false, None),
                ],
            ),
            group(
                2,
                vec![member("d", 10, true, None), member("e", 10, false, None)],
            ),
        ],
        selected: [PathBuf::from("b"), PathBuf::from("e")]
            .into_iter()
            .collect(),
        ..AppState::default()
    };
    let order: Vec<PathBuf> = s.groups[0].members.iter().map(|m| m.path.clone()).collect();
    actions::set_keeper_in(&mut s, 1, Path::new("c"));
    let after: Vec<PathBuf> = s.groups[0].members.iter().map(|m| m.path.clone()).collect();
    assert_eq!(order, after, "members keep their order");
    assert!(
        s.groups[0]
            .members
            .iter()
            .any(|m| m.path == Path::new("c") && m.keep),
        "the picked file becomes the keeper"
    );
    assert!(
        s.groups[0]
            .members
            .iter()
            .all(|m| m.path == Path::new("c") || !m.keep),
        "only one keeper per group"
    );
    // The retiring keeper becomes a ticked duplicate; the other group is
    // untouched, so a pick never re-ticks the whole list.
    assert!(s.selected.contains(Path::new("b")));
    assert!(!s.selected.contains(Path::new("c")));
    assert!(s.selected.contains(Path::new("e")));
}

#[test]
fn confirm_plan_caps_rows_but_keeps_its_counts_exact() {
    let mut members = vec![member("keep.txt", 10, true, None)];
    for i in 0..8 {
        members.push(member(&format!("dup-{i}.txt"), 10, false, None));
    }
    let s = AppState {
        groups: vec![group(1, members.clone())],
        selected: members
            .iter()
            .filter(|m| !m.keep)
            .map(|m| m.path.clone())
            .collect(),
        ..AppState::default()
    };
    let (groups, hidden_groups) = actions::confirm_groups(&s);
    assert_eq!(hidden_groups, 0);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].keeper, "keep.txt");
    assert_eq!(groups[0].rows.len(), actions::CONFIRM_ROWS);
    assert_eq!(groups[0].hidden_rows, 8 - actions::CONFIRM_ROWS);
    assert_eq!(groups[0].bytes, 80);
}

/// The Delete button asks first: the confirmation names the keeper and the
/// file that would go, and Cancel leaves every file in place.
#[test]
fn delete_asks_for_confirmation_before_removing_anything() {
    let (mut app, _guard) = start();
    let dir = std::env::temp_dir().join(format!(
        "dedupe-confirm-fixture-{}-{}",
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
    let result = (|| {
        app.wait_for("text:Scan a folder")?;
        let mut pasted = nsh_core::PasteEvent::default();
        pasted.text = dir.display().to_string();
        app.paste("text:Scan a folder", &pasted)?;
        app.press("label:Add folder")?;
        app.wait_for("label:Remove folder")?;
        app.press("text:Scan for duplicates")?;
        app.wait_for_within("text:/group\\(s\\)/", std::time::Duration::from_secs(60))?;
        // Both copies are 4096 bytes, so the button names its exact count.
        app.wait_for("text:Delete 1 (4.10 KB)")?;
        app.press("text:Delete 1 (4.10 KB)")?;
        app.wait_for("text:Delete 1 file(s)?")?;
        app.wait_for(r"text:/keeps (one|two)\.txt/")?;
        app.press("text:Cancel")?;
        // Cancel dismisses it without touching the files.
        let mut closed = false;
        for _ in 0..40 {
            if !app.shows("Delete 1 file(s)?") {
                closed = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if !closed {
            return Err("Cancel did not dismiss the confirmation".to_string());
        }
        if !(dir.join("one.txt").is_file() && dir.join("two.txt").is_file()) {
            return Err("nothing may be removed before the delete is confirmed".to_string());
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result.expect("the delete confirmation asks first and cancels cleanly");
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
        app.wait_for("text:Scan a folder")?;
        let mut pasted = nsh_core::PasteEvent::default();
        pasted.text = dir.display().to_string();
        app.paste("text:Scan a folder", &pasted)?;
        app.press("label:Add folder")?;
        app.wait_for("label:Remove folder")?;
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
        app.wait_for("text:Scan a folder")?;
        let mut pasted = nsh_core::PasteEvent::default();
        pasted.text = dir.display().to_string();
        app.paste("text:Scan a folder", &pasted)?;
        app.press("label:Add folder")?;
        app.wait_for("label:Remove folder")?;
        app.press("text:Scan for duplicates")?;
        app.wait_for_within("text:/group\\(s\\)/", std::time::Duration::from_secs(60))?;
        // The member rows themselves, not just the summary chip.
        app.wait_for("text:/one\\.png/")
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result.expect("media groups render with their rows");
}

/// Every duplicate group starts expanded: after a scan both groups' members
/// are on screen without pressing Expand first.
#[test]
fn every_group_starts_expanded() {
    let (mut app, _guard) = start();
    let dir = std::env::temp_dir().join(format!(
        "dedupe-expanded-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    for (name, byte) in [
        ("a1.txt", b'a'),
        ("a2.txt", b'a'),
        ("b1.txt", b'b'),
        ("b2.txt", b'b'),
    ] {
        std::fs::write(dir.join(name), vec![byte; 4096]).expect("fixture file");
    }
    let result = (|| {
        app.wait_for("text:Scan a folder")?;
        let mut pasted = nsh_core::PasteEvent::default();
        pasted.text = dir.display().to_string();
        app.paste("text:Scan a folder", &pasted)?;
        app.press("label:Add folder")?;
        app.wait_for("label:Remove folder")?;
        app.press("text:Scan for duplicates")?;
        app.wait_for_within("text:/2 group\\(s\\)/", std::time::Duration::from_secs(60))?;
        // Both groups' rows render with no Expand press in between.
        app.wait_for("text:/a1\\.txt/")?;
        app.wait_for("text:/b1\\.txt/")
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result.expect("every group renders expanded after a scan");
}

/// A video preview plays in place and stops again: the card swaps in the
/// muted player and the Stop pill puts it back, without leaving the list.
#[test]
fn video_preview_plays_and_stops() {
    let (mut app, _guard) = start();
    let dir = std::env::temp_dir().join(format!(
        "dedupe-video-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    std::fs::write(dir.join("one.mp4"), vec![7u8; 4096]).expect("fixture video");
    std::fs::copy(dir.join("one.mp4"), dir.join("two.mp4")).expect("copy fixture video");
    let result = (|| {
        app.wait_for("text:Scan a folder")?;
        let mut pasted = nsh_core::PasteEvent::default();
        pasted.text = dir.display().to_string();
        app.paste("text:Scan a folder", &pasted)?;
        app.press("label:Add folder")?;
        app.wait_for("label:Remove folder")?;
        app.press("text:Scan for duplicates")?;
        app.wait_for_within("text:/group\\(s\\)/", std::time::Duration::from_secs(60))?;
        // A video group offers the inline player: both cards have one, and
        // pressing either starts that card's clip.
        app.wait_for("label:Play video")?;
        app.press("label:Play video >> nth=0")?;
        app.wait_for("label:Stop video")?;
        app.press("label:Stop video >> nth=0")?;
        // Stopping returns the card to its poster/placeholder state.
        for _ in 0..60 {
            if app.screen().count("label:Stop video").unwrap_or(0) == 0 {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        Err("Stop left the preview playing".to_string())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result.expect("video previews play and stop in place");
}

/// Photos carry no player: the Play/Stop control and the play glyph belong to
/// videos, so an image card never offers an action with nothing behind it.
#[test]
fn image_cards_have_no_play_button() {
    let (mut app, _guard) = start();
    let dir = std::env::temp_dir().join(format!(
        "dedupe-image-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    std::fs::write(dir.join("one.png"), vec![1u8; 4096]).expect("fixture image");
    std::fs::copy(dir.join("one.png"), dir.join("two.png")).expect("copy fixture image");
    let result = (|| {
        app.wait_for("text:Scan a folder")?;
        let mut pasted = nsh_core::PasteEvent::default();
        pasted.text = dir.display().to_string();
        app.paste("text:Scan a folder", &pasted)?;
        app.press("label:Add folder")?;
        app.wait_for("label:Remove folder")?;
        app.press("text:Scan for duplicates")?;
        app.wait_for_within("text:/group\\(s\\)/", std::time::Duration::from_secs(60))?;
        app.wait_for("text:/one\\.png/")?;
        // Let a wrongly offered control show up before asserting it is absent.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let players = app.screen().count("label:Play video").unwrap_or(0)
            + app.screen().count("label:Stop video").unwrap_or(0);
        if players > 0 {
            return Err("an image card offers a play control".to_string());
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result.expect("image cards render without play controls");
}

/// Grid is the original layout, List is the opt-in compact one: switching
/// re-lays the same results and keeps the group open and its members listed.
#[test]
fn layout_switch_keeps_the_results_listed() {
    let (mut app, _guard) = start();
    let dir = std::env::temp_dir().join(format!(
        "dedupe-layout-fixture-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("fixture dir");
    std::fs::write(dir.join("one.png"), vec![2u8; 4096]).expect("fixture image");
    std::fs::copy(dir.join("one.png"), dir.join("two.png")).expect("copy fixture image");
    let result = (|| {
        app.wait_for("text:Scan a folder")?;
        let mut pasted = nsh_core::PasteEvent::default();
        pasted.text = dir.display().to_string();
        app.paste("text:Scan a folder", &pasted)?;
        app.press("label:Add folder")?;
        app.wait_for("label:Remove folder")?;
        app.press("text:Scan for duplicates")?;
        app.wait_for_within("text:/group\\(s\\)/", std::time::Duration::from_secs(60))?;
        app.wait_for("text:/one\\.png/")?;
        // List, then back to Grid: the same two members stay listed.
        app.press("label:List layout")?;
        app.wait_for("text:/one\\.png/")?;
        app.wait_for("text:/two\\.png/")?;
        app.press("label:Grid layout")?;
        app.wait_for("text:/two\\.png/")
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result.expect("the layout switch keeps the results listed");
}
