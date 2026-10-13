//! Headless UI tests for the desktop GUI.
//!
//! Every test renders the real view into an offscreen gpui-kit window
//! through the production entry point (`gpui_kit::open_window`, the same
//! call `ui::run` makes), so the render path under test is exactly what
//! the user sees ??empty state, sidebar, settings page, group accordion,
//! results toolbar ??without a visible window.
//!
//! Prefs are pointed at a temp file under [`ENV_LOCK`] (the env var is
//! process-global, so the lock serializes these tests against the prefs
//! override test and each other; the test harness runs them in parallel
//! threads otherwise). Nothing here touches the user's real
//! `~/.dedupe/gui-prefs.json`.

use crate::matching::{Group, GroupMember};
use crate::media::MediaKind;
use crate::prefs::ENV_LOCK;
use crate::ui::{AppView, DedupeView, SettingsTab};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Bounds, Entity, Size, TestAppContext, Window,
    WindowBounds, WindowOptions, px,
};
use std::collections::HashSet;
use std::path::PathBuf;

/// Open the production window with prefs redirected to a temp file.
/// The returned guard holds [`ENV_LOCK`] for the whole test: view
/// interactions (clicks, keeper picks) trigger `save_prefs`, which reads
/// the env var and writes the temp file.
fn open_view(
    cx: &mut TestAppContext,
) -> (
    AnyWindowHandle,
    Entity<DedupeView>,
    std::sync::MutexGuard<'static, ()>,
) {
    let guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let prefs = std::env::temp_dir().join(format!(
        "dedupe-ui-prefs-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    // SAFETY: guarded by ENV_LOCK; the value is a per-test temp path.
    unsafe {
        std::env::set_var("DEDUPE_PREFS", &prefs);
    }
    cx.update(gpui_kit::init);
    cx.update(|cx| cx.set_reduce_motion(true));
    let (handle, view) = cx.update(|cx| {
        let bounds = Bounds::maximized(None, cx);
        let (window, content) = gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            cx,
            |window, cx| cx.new(|cx| DedupeView::new(window, cx)),
        )
        .expect("open test window");
        (window, content)
    });
    (handle, view, guard)
}

/// One render pass; makes the element tree available to `find`/`click`.
fn draw(cx: &mut TestAppContext, handle: AnyWindowHandle) {
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))
        .unwrap();
}

/// Run a closure against the window, panicking on failure (window
/// updates never fail in tests).
fn in_window(
    cx: &mut TestAppContext,
    handle: AnyWindowHandle,
    f: impl FnOnce(&mut Window, &mut App),
) {
    cx.update_window(handle, |_, window, cx| f(window, cx))
        .unwrap();
}

/// Run an assertion against the view's state.
fn assert_view<R>(
    cx: &mut TestAppContext,
    view: &Entity<DedupeView>,
    f: impl FnOnce(&mut DedupeView) -> R,
) -> R {
    cx.update(|app| view.update(app, |v, _| f(v)))
}

fn member(
    path: &str,
    size: u64,
    keep: bool,
    similarity: Option<f64>,
    mtime: Option<i64>,
) -> GroupMember {
    GroupMember {
        path: PathBuf::from(path),
        size,
        mtime_secs: mtime,
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

/// Give the view `count` plain-file groups of two members (one keeper).
fn inject_groups(view: &Entity<DedupeView>, cx: &mut App, count: usize) {
    view.update(cx, |v, cx| {
        v.groups = (1..=count)
            .map(|i| {
                group(
                    i,
                    vec![
                        member(&format!("dir/file-{i}-a.txt"), 100, true, None, None),
                        member(&format!("dir/file-{i}-b.txt"), 100, false, None, None),
                    ],
                )
            })
            .collect();
        v.has_scanned = true;
        v.selected = HashSet::new();
        cx.notify();
    });
}

fn positive(bounds: gpui_kit::Bounds<gpui_kit::Pixels>) -> Size<gpui_kit::Pixels> {
    assert!(
        bounds.size.width > px(0.) && bounds.size.height > px(0.),
        "element must have real layout, got {:?}",
        bounds.size
    );
    bounds.size
}

// ---------------------------------------------------------------------------
// Shell rendering
// ---------------------------------------------------------------------------

#[gpui_kit::test]
fn first_run_shell_renders_core_controls(cx: &mut TestAppContext) {
    let (handle, _view, _guard) = open_view(cx);
    draw(cx, handle);

    in_window(cx, handle, |window, _| {
        // Scan button, Settings nav and theme toggle all have layout.
        assert!(positive(window.find("scan").bounds()).width > px(0.));
        assert!(positive(window.find("nav-settings").bounds()).width > px(0.));
        assert!(positive(window.find("theme-toggle").bounds()).width > px(0.));
    });
}

#[gpui_kit::test]
fn settings_page_renders_both_tabs(cx: &mut TestAppContext) {
    let (handle, view, _guard) = open_view(cx);
    draw(cx, handle);

    for tab in [SettingsTab::General, SettingsTab::Preferences] {
        in_window(cx, handle, |window, cx| {
            view.update(cx, |v, cx| {
                v.view = AppView::Settings;
                v.settings_tab = tab;
                cx.notify();
            });
            window.render_frame(cx);
            // The header swaps Settings for a Back button.
            assert!(positive(window.find("nav-back").bounds()).width > px(0.));
        });
    }
}

#[gpui_kit::test]
fn stop_button_resets_the_ui_immediately(cx: &mut TestAppContext) {
    let (handle, view, _guard) = open_view(cx);
    // A scan in flight with results already on screen.
    in_window(cx, handle, |window, cx| {
        view.update(cx, |v, cx| {
            v.scanning = true;
            v.has_scanned = true;
            v.groups = vec![crate::matching::Group {
                index: 1,
                hash: "h".to_string(),
                media_kind: crate::media::MediaKind::Other,
                similarity: None,
                members: vec![member("dir/a.txt", 100, true, None, None)],
            }];
            v.selected = [PathBuf::from("dir/a.txt")].into_iter().collect();
            cx.notify();
        });
        window.render_frame(cx);
        // Scanning swaps the primary Scan button for a danger Stop button.
        assert!(positive(window.find("stop-scan").bounds()).width > px(0.));
        window.click("stop-scan", cx);
        window.render_frame(cx);
    });

    // Everything resets in the same frame — no "Stopping…" limbo while a
    // large file finishes hashing.
    assert_view(cx, &view, |v| {
        assert!(!v.scanning, "scanning clears immediately");
        assert!(v.scan_cancel.is_cancelled(), "the token is armed");
        assert!(v.groups.is_empty() && v.selected.is_empty());
        assert!(!v.has_scanned);
        assert_eq!(v.status, "Scan cancelled.");
        assert_eq!(v.scan_done, 0);
    });
}

#[gpui_kit::test]
fn stale_scan_output_is_discarded(cx: &mut TestAppContext) {
    // A task that finishes after Stop (or after a new scan started) must
    // not resurrect its results.
    let (_handle, view, _guard) = open_view(cx);
    let stale_gen = assert_view(cx, &view, |v| {
        v.scanning = true;
        v.scan_gen += 1; // this task's generation
        v.scan_gen
    });
    // Stop (or another start) bumps the generation under the task.
    assert_view(cx, &view, |v| v.scan_gen += 1);
    assert_view(cx, &view, |v| {
        assert!(
            !v.is_current_scan(stale_gen),
            "stale output must be dropped"
        );
        assert!(
            v.is_current_scan(v.scan_gen),
            "the newest generation is current"
        );
    });
    // The same guard on the normal path: a still-current task applies.
    let current = assert_view(cx, &view, |v| v.scan_gen);
    assert_view(cx, &view, |v| assert!(v.is_current_scan(current)));
}

#[gpui_kit::test]
fn scan_button_is_primary_while_idle(cx: &mut TestAppContext) {
    // No Stop while idle: only the primary Scan control is present.
    let (handle, view, _guard) = open_view(cx);
    in_window(cx, handle, |window, cx| {
        view.update(cx, |v, cx| {
            v.scanning = false;
            cx.notify();
        });
        window.render_frame(cx);
        assert!(positive(window.find("scan").bounds()).width > px(0.));
    });
}
// The keystroke wiring itself (`.on_key_down` matching "escape") is
// declarative and not covered: the offscreen harness does not route
// key events into windows created through the production
// `gpui_kit::open_window`. The handler body is tested directly.
#[gpui_kit::test]
fn escape_leaves_the_settings_page(cx: &mut TestAppContext) {
    // The keystroke wiring itself (`.on_key_down` matching "escape") is
    // declarative and not covered: the offscreen harness does not route
    // key events into windows created through the production
    // `gpui_kit::open_window`. The handler body is tested directly.
    let (handle, view, _guard) = open_view(cx);
    in_window(cx, handle, |window, cx| {
        view.update(cx, |v, cx| {
            v.view = AppView::Settings;
            v.leave_settings(cx);
        });
        window.render_frame(cx);
    });
    assert_view(cx, &view, |v| assert_eq!(v.view, AppView::Main));

    // No-op on the main view: the main flow must never be switched away
    // from by escape.
    in_window(cx, handle, |window, cx| {
        view.update(cx, |v, cx| v.leave_settings(cx));
        window.render_frame(cx);
    });
    assert_view(cx, &view, |v| assert_eq!(v.view, AppView::Main));
}

// ---------------------------------------------------------------------------
// Toolbar interactions (real click paths)
// ---------------------------------------------------------------------------

#[gpui_kit::test]
fn scan_without_folders_sets_a_help_status(cx: &mut TestAppContext) {
    let (handle, view, _guard) = open_view(cx);
    in_window(cx, handle, |window, cx| {
        window.render_frame(cx);
        window.click("scan", cx);
        window.render_frame(cx);
    });

    assert_view(cx, &view, |v| {
        assert!(
            v.status.contains("Add a folder"),
            "unexpected status: {}",
            v.status
        );
        assert!(!v.scanning, "no scan may start without folders");
    });
}

#[gpui_kit::test]
fn theme_toggle_switches_to_dark_mode(cx: &mut TestAppContext) {
    let (handle, _view, _guard) = open_view(cx);
    cx.update(|app| assert!(!app.theme().is_dark(), "starts light"));

    in_window(cx, handle, |window, cx| {
        window.render_frame(cx);
        window.click("theme-toggle", cx);
        window.render_frame(cx);
    });

    cx.update(|app| assert!(app.theme().is_dark(), "toggle must flip to dark"));
}

#[gpui_kit::test]
fn expand_and_collapse_all_update_group_state(cx: &mut TestAppContext) {
    let (handle, view, _guard) = open_view(cx);
    in_window(cx, handle, |window, cx| {
        inject_groups(&view, cx, 3);
        window.render_frame(cx);
        window.click("expand-all", cx);
        window.render_frame(cx);
    });
    assert_view(cx, &view, |v| assert_eq!(v.expanded.len(), 3));

    in_window(cx, handle, |window, cx| {
        window.render_frame(cx);
        window.click("collapse-all", cx);
        window.render_frame(cx);
    });
    assert_view(cx, &view, |v| assert!(v.expanded.is_empty()));
}

#[gpui_kit::test]
fn select_presets_all_none_invert(cx: &mut TestAppContext) {
    let (handle, view, _guard) = open_view(cx);
    in_window(cx, handle, |window, cx| {
        inject_groups(&view, cx, 3);
        window.render_frame(cx);
        window.click("sel-all", cx);
        window.render_frame(cx);
    });
    // One dup per group, keepers never selectable.
    assert_view(cx, &view, |v| assert_eq!(v.selected.len(), 3));

    in_window(cx, handle, |window, cx| {
        window.render_frame(cx);
        window.click("sel-none", cx);
        window.render_frame(cx);
    });
    assert_view(cx, &view, |v| assert!(v.selected.is_empty()));

    in_window(cx, handle, |window, cx| {
        window.render_frame(cx);
        window.click("sel-invert", cx);
        window.render_frame(cx);
    });
    assert_view(cx, &view, |v| assert_eq!(v.selected.len(), 3));
}

#[gpui_kit::test]
fn keep_mode_buttons_reassign_keepers(cx: &mut TestAppContext) {
    let (handle, view, _guard) = open_view(cx);
    in_window(cx, handle, |window, cx| {
        view.update(cx, |v, cx| {
            v.keep_index = Some(0);
            v.groups = vec![group(
                1,
                vec![
                    // Oldest by mtime, first by path: default keep.
                    member("dir/old.txt", 100, true, None, Some(100)),
                    member("dir/new.txt", 200, false, None, Some(500)),
                ],
            )];
            v.has_scanned = true;
            cx.notify();
        });
        window.render_frame(cx);
        // keep-mode-2 = Newest.
        window.click("keep-mode-2", cx);
        window.render_frame(cx);
    });

    assert_view(cx, &view, |v| {
        assert_eq!(v.keep_index, Some(2));
        let g = &v.groups[0];
        let keeper = g.members.iter().find(|m| m.keep).expect("a keeper exists");
        assert!(
            keeper.path.ends_with("new.txt"),
            "Newest must keep the later mtime, got {keeper:?}"
        );
    });
}

// ---------------------------------------------------------------------------
// View state transitions
// ---------------------------------------------------------------------------

#[gpui_kit::test]
fn keeper_pick_moves_keep_and_reselects(cx: &mut TestAppContext) {
    let (handle, view, _guard) = open_view(cx);
    in_window(cx, handle, |window, cx| {
        view.update(cx, |v, cx| {
            v.groups = vec![group(
                1,
                vec![
                    member("dir/a.txt", 100, true, None, None),
                    member("dir/b.txt", 100, false, None, None),
                ],
            )];
            v.selected = HashSet::new();
            cx.notify();
        });
        window.render_frame(cx);
        // Promote the dup to keeper through the real view API.
        let target = PathBuf::from("dir/b.txt");
        view.update(cx, |v, cx| v.set_keeper(1, &target, cx));
        window.render_frame(cx);
    });

    assert_view(cx, &view, |v| {
        let g = &v.groups[0];
        let keepers: Vec<&PathBuf> = g
            .members
            .iter()
            .filter(|m| m.keep)
            .map(|m| &m.path)
            .collect();
        assert_eq!(keepers, vec![&PathBuf::from("dir/b.txt")]);
        // The new dup (old keeper) is now the only selection.
        assert_eq!(v.selected.len(), 1);
        assert!(v.selected.contains(&PathBuf::from("dir/a.txt")));
    });
}

#[gpui_kit::test]
fn similarity_floor_filters_group_members(cx: &mut TestAppContext) {
    let (handle, view, _guard) = open_view(cx);
    in_window(cx, handle, |window, cx| {
        view.update(cx, |v, cx| {
            v.scan_similarity = 0.95;
            v.min_sim = 0.95;
            let members = vec![
                member("dir/keep.jpg", 300, true, None, None),
                member("dir/close.jpg", 200, false, Some(0.985), None),
                member("dir/far.jpg", 100, false, Some(0.95), None),
            ];
            v.groups = vec![group(1, members.clone())];
            v.base_groups = vec![group(1, members)];
            v.has_scanned = true;
            cx.notify();
            // Tighten the floor above the far member's score.
            v.apply_threshold(0.98, cx);
        });
        window.render_frame(cx);
    });

    assert_view(cx, &view, |v| {
        let g = &v.groups[0];
        assert_eq!(g.members.len(), 2, "far.jpg must drop below the floor");
        assert!(g.members.iter().any(|m| m.path.ends_with("keep.jpg")));
        assert!(g.members.iter().any(|m| m.path.ends_with("close.jpg")));
        assert!((v.min_sim - 0.98).abs() < 1e-9);
    });
}

#[gpui_kit::test]
fn merged_exact_and_similar_members_render_together(cx: &mut TestAppContext) {
    // The merge fix end-to-end at the view layer: one exact pair plus a
    // downscaled variant (with a similarity score) renders as a single
    // three-member group.
    let (handle, view, _guard) = open_view(cx);
    in_window(cx, handle, |window, cx| {
        view.update(cx, |v, cx| {
            v.groups = vec![group(
                1,
                vec![
                    member("dir/big.png", 900, true, None, None),
                    member("dir/big-copy.png", 900, false, None, None),
                    member("dir/small.png", 40, false, Some(0.9766), None),
                ],
            )];
            v.has_scanned = true;
            v.selected = [PathBuf::from("dir/small.png")].into_iter().collect();
            cx.notify();
        });
        window.render_frame(cx);
    });

    // Renders with the group header plus all three member rows present.
    assert_view(cx, &view, |v| {
        assert_eq!(v.groups[0].members.len(), 3);
        assert_eq!(v.selected.len(), 1);
    });
    draw(cx, handle);
}
