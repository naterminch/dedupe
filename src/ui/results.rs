//! Main results pane: first-run guidance, live progress, duplicate groups and
//! cleanup findings.
//!
//! Group rows are built inside one `dynamic` region: every read of [`app`]
//! subscribes, so a scan, a keeper swap or a tick re-renders the list.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use nsh_core::prelude::*;

use crate::matching::{Group, GroupMember, KeepReason};
use crate::media;
use crate::modes::mode_title;
use crate::ui::actions;
use crate::ui::palette::{size, space};
use crate::ui::state::{
    AppState, Cleared, FindingsDone, FindingsWent, Layout, Removal, app, scan_status,
};
use crate::ui::widgets::{
    BadgeTone, Decision, Ink, Kind, PreviewFill, PreviewSize, SelectItem, action_bar, action_group,
    alert_dialog, amount, arriving, badge, body, caption, card, clip_glyph, count_up,
    danger_button, description, disclosure, divider, dropdown_trigger, entering, error_notice,
    fading_in, fast_scroll, fast_scroll_by, fast_scroll_max, field, field_label, figure,
    ghost_button, heading, hint, icon_button, icon_ghost_button, icon_secondary_button,
    icon_segmented, icons, ink_lift, keeper_badge, kind_preview, name, notice, option_select,
    panel_title, press_to_play, pressed_seal, preview_frame, preview_image, primary_button,
    progress_bar, quiet_button, radio_segmented, secondary_button, section_title, segmented, tick,
    tile, title_block, turning_enso, video_player,
};
use crate::ui_helpers::{counted, fit_text};
use crate::util;

const KEEP_MODES: [&str; 5] = ["First", "Smallest", "Newest", "Oldest", "Best"];

/// Photo extensions the image element decodes. Formats it cannot (HEIC, SVG,
/// ICO, TIFF) keep the text-only row rather than a broken box.
const THUMB_EXTS: [&str; 6] = ["jpg", "jpeg", "png", "gif", "bmp", "webp"];

/// Extensions read as documents and as archives: only to choose a group's
/// kind tag, so a miss reads as a plain "File", never as a wrong action.
const DOCUMENT_EXTS: [&str; 18] = [
    "pdf", "doc", "docx", "odt", "rtf", "txt", "md", "pages", "xls", "xlsx", "ods", "csv",
    "numbers", "ppt", "pptx", "odp", "key", "epub",
];
const ARCHIVE_EXTS: [&str; 13] = [
    "zip", "7z", "rar", "tar", "gz", "tgz", "bz2", "xz", "zst", "iso", "dmg", "cab", "msi",
];

/// What sort of file a path is, for its kind tag and its preview's dye.
fn file_kind(path: &Path) -> Kind {
    match media::classify(path) {
        media::MediaKind::Image => return Kind::Photo,
        media::MediaKind::Video => return Kind::Video,
        media::MediaKind::Other => {}
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let ext = ext.as_str();
    if crate::modes::AUDIO_EXTS.contains(&ext) {
        Kind::Audio
    } else if DOCUMENT_EXTS.contains(&ext) {
        Kind::Document
    } else if ARCHIVE_EXTS.contains(&ext) {
        Kind::Archive
    } else {
        Kind::File
    }
}

/// The word on a file kind's tag.
fn kind_label(kind: Kind) -> &'static str {
    match kind {
        Kind::Photo => "Photo",
        Kind::Video => "Video",
        Kind::Audio => "Audio",
        Kind::Document => "Document",
        Kind::Archive => "Archive",
        _ => "File",
    }
}

/// The dye a cleanup finding's kind takes; its tag keeps the finding's own
/// word.
fn finding_kind(kind: &str) -> Kind {
    match kind {
        "empty" | "empty-dir" | "symlink" => Kind::Empty,
        "temp" => Kind::Leftover,
        "broken" | "bad-extension" | "bad-name" => Kind::Damaged,
        "big" => Kind::Large,
        "metadata" => Kind::Metadata,
        _ => Kind::File,
    }
}

/// Height of one group card, which is what one wheel notch moves: a photo card
/// in grid layout carries a large preview, a compact card two plain rows.
const CARD_STEP_GRID: f32 = 355.0;
const CARD_STEP_LIST: f32 = 240.0;

/// What preview a member row shows: the photo itself, a video's extracted
/// poster frame, a placeholder while that poster extracts, or nothing for
/// plain files.
enum Thumb {
    None,
    Image,
    Poster(PathBuf),
    PendingVideo,
}

/// Decide the preview for one member.
///
/// Videos cannot be decoded by the UI toolkit, so they show a poster frame
/// ffmpeg extracts into the shared state; until it lands the row shows a
/// placeholder. Photos render straight from the file path.
fn thumb_for(member: &GroupMember, poster: Option<&Path>) -> Thumb {
    match media::classify(&member.path) {
        media::MediaKind::Image => {
            // Only files that are still on disk become image sources: a
            // member whose file vanished (deleted in another app, an
            // unmounted drive) keeps the text row instead of asking the
            // backend to load a path that is not there.
            let thumbable = member
                .path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| THUMB_EXTS.contains(&e.to_ascii_lowercase().as_str()))
                && member.path.is_file();
            if thumbable { Thumb::Image } else { Thumb::None }
        }
        // The poster frame (when ffmpeg extracted one) is part of the card's
        // own signature, so it swaps in without the group rebuilding.
        media::MediaKind::Video => poster.map_or(Thumb::PendingVideo, |frame| {
            Thumb::Poster(frame.to_path_buf())
        }),
        media::MediaKind::Other => Thumb::None,
    }
}

/// The preview of one copy: the photo itself, a video's poster, or a glyph
/// for files that have none.
fn member_preview(thumb: &Thumb, path: &Path, size: PreviewSize) -> View {
    match thumb {
        Thumb::Image => {
            preview_frame(size, PreviewFill::Picture).child(preview_image(path_source(path), size))
        }
        Thumb::Poster(poster) => preview_frame(size, PreviewFill::Picture)
            .child(preview_image(path_source(poster), size)),
        // A file with no preview gets a slip of its kind's dye in a row, not
        // an empty box: the box read as a thumbnail that failed to load. On a
        // card it is a photo the image element cannot decode, so the box
        // stays plain.
        Thumb::None => match size {
            PreviewSize::Card => preview_frame(size, PreviewFill::Empty),
            PreviewSize::Thumb => kind_preview(file_kind(path)),
        },
        Thumb::PendingVideo => preview_frame(size, PreviewFill::Empty).child(clip_glyph()),
    }
}

/// A path as an image or video source.
fn path_source(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

pub(crate) fn results() -> View {
    // On the root ground: transparent, so its grain runs on under the pane.
    view()
        .flex_col()
        .flex_1()
        .h_full()
        .min_w(px(0.0))
        .overflow_hidden()
        .child(dynamic(|| {
            // Which workspace is up, not its progress: a scan tick must not
            // swap these panes out from under a press.
            let mode = computed(|| app().with(|s| s.mode_result.is_some()));
            if mode.get() {
                mode_workspace()
            } else {
                duplicate_workspace()
            }
        }))
}

// Where each long list sits, as the wheel last left it. The wheel is the
// app's over these lists (`fast_scroll`): `scroll_to` takes an absolute
// offset, so the pane keeps the one it moved to.
thread_local! {
    static RESULTS_AT: Cell<(f32, f32, f32)> = const { Cell::new((0.0, f32::NAN, 0.0)) };
    static MODE_AT: Cell<(f32, f32, f32)> = const { Cell::new((0.0, f32::NAN, 0.0)) };
    static DELETE_AT: Cell<(f32, f32, f32)> = const { Cell::new((0.0, f32::NAN, 0.0)) };
    static CONSOLIDATE_AT: Cell<(f32, f32, f32)> = const { Cell::new((0.0, f32::NAN, 0.0)) };
    static FINDINGS_AT: Cell<(f32, f32, f32)> = const { Cell::new((0.0, f32::NAN, 0.0)) };
}

/// Puts the delete dialog's list back at its top: it opens anew each time,
/// while the wheel's remembered offset outlives it.
pub(crate) fn reset_delete_scroll() {
    DELETE_AT.with(|cell| cell.set((0.0, f32::NAN, 0.0)));
}

/// [`reset_delete_scroll`] for the consolidate confirmation's list.
pub(crate) fn reset_consolidate_scroll() {
    CONSOLIDATE_AT.with(|cell| cell.set((0.0, f32::NAN, 0.0)));
}

/// [`reset_delete_scroll`] for the cleanup findings' confirmation.
pub(crate) fn reset_findings_scroll() {
    FINDINGS_AT.with(|cell| cell.set((0.0, f32::NAN, 0.0)));
}

/// Puts the workspace's lists back at their top: leaving the page unmounts
/// them, so the wheel's remembered offsets would point into a list that opens
/// at the start again.
pub(crate) fn reset_lists() {
    for at in [
        &RESULTS_AT,
        &MODE_AT,
        &DELETE_AT,
        &CONSOLIDATE_AT,
        &FINDINGS_AT,
    ] {
        at.with(|cell| cell.set((0.0, f32::NAN, 0.0)));
    }
}

fn duplicate_workspace() -> View {
    view()
        .flex_col()
        .flex_1()
        .h_full()
        .gap(px(space::XL))
        .px(px(space::MA))
        .py(px(space::XXL))
        // Before the first scan the workspace is the setup form, which brings
        // its own title row. Once a scan runs the results header and toolbars
        // take over until Back returns to the form.
        .child(dynamic(|| {
            // The toolbar follows the page's shape, not its progress: reading
            // the app signal directly rebuilt these buttons on every scan tick,
            // which restarted their hover and dropped presses.
            let shape =
                computed(|| app().with(|s| (s.scanning, s.has_scanned, !s.groups.is_empty())));
            let (scanning, has_scanned, has_groups) = shape.get();
            if scanning || has_scanned {
                view()
                    .flex_col()
                    .gap(px(space::LG))
                    .child(header_row())
                    .child(if has_groups {
                        action_row()
                    } else {
                        back_only_row()
                    })
            } else {
                view()
            }
        }))
        // Rebuild only when the results change shape: a scan, a threshold, a
        // delete, a consolidate, a re-sort, Show more. Keeper picks and ticks
        // update their own cards, so pressing Keep never rebuilds (or
        // reorders) the list. When a delete clears the last group, the cards
        // lift off the paper and the outcome is inked in behind them.
        .child(ink_lift(
            || {
                app().with(|s| {
                    s.has_scanned
                        && !s.scanning
                        && s.groups.is_empty()
                        && matches!(s.cleared, Some(Cleared::Removed { .. }))
                })
            },
            fast_scroll_by(
                &RESULTS_AT,
                card_step,
                dynamic(|| {
                    let signature = computed(|| {
                        app().with(|s| {
                            (
                                s.results_rev,
                                s.scanning,
                                s.has_scanned,
                                s.groups.len(),
                                s.group_limit,
                                s.ffprobe_note,
                            )
                        })
                    });
                    list_body(signature.get())
                }),
            ),
        ))
}

fn header_row() -> View {
    view()
        .flex_row()
        .flex_wrap()
        .justify_between()
        .items_end()
        .gap(px(space::LG))
        .child(title_block(
            heading(|| {
                app().with(|s| {
                    if s.scanning {
                        "Scanning"
                    } else if s.groups.is_empty() && s.cleared.is_some() {
                        "Duplicates cleared"
                    } else {
                        "Review duplicates"
                    }
                })
            }),
            summary_line(),
        ))
        .child(dynamic(|| {
            let shown = computed(|| app().with(answer));
            match shown.get() {
                Answer::Waiting => view(),
                Answer::Reclaimable => reclaim_figure(),
                Answer::Removed {
                    trashed,
                    freed,
                    moved,
                } => removed_figure(trashed, freed, moved),
            }
        }))
}

/// What the header's figure answers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Answer {
    /// No figure: the scan is running (a count of bytes found so far would
    /// read as the result), it found nothing (a vermilion "0 B" would shout
    /// over the card that says so), or copies came back from the trash and
    /// the numbers wait for the next scan.
    Waiting,
    /// The space the marked copies would give back.
    Reclaimable,
    /// Every group is gone: what the deletes since the scan took, split by
    /// where it went.
    Removed {
        trashed: u64,
        freed: u64,
        moved: u64,
    },
}

fn answer(s: &AppState) -> Answer {
    if !s.has_scanned || s.scanning {
        return Answer::Waiting;
    }
    match s.cleared {
        Some(Cleared::Removed {
            trashed,
            freed,
            moved,
            ..
        }) if s.groups.is_empty() => Answer::Removed {
            trashed,
            freed,
            moved,
        },
        Some(Cleared::Restored { .. }) if s.groups.is_empty() => Answer::Waiting,
        None if s.groups.is_empty() => Answer::Waiting,
        _ => Answer::Reclaimable,
    }
}

/// The figure once every group is cleared. Space moved to the trash is not
/// free until the trash is emptied, so it is never called freed: a trash
/// delete reads "In the trash", a permanent one (or a hard link) "Freed", and
/// a mix names the freed space with the way to the rest under it. Copies only
/// moved into another folder free nothing, so a run of moves alone reads
/// "Moved aside".
fn removed_figure(trashed: u64, freed: u64, moved: u64) -> View {
    if freed == 0 && trashed == 0 {
        return answer_figure("Moved aside", move || util::human_bytes(moved));
    }
    if freed == 0 {
        return answer_figure("In the trash", move || util::human_bytes(trashed)).child(caption(
            format!("Empty the trash to get {} back", util::human_bytes(trashed)),
        ));
    }
    let figure = answer_figure("Freed", move || util::human_bytes(freed));
    if trashed == 0 {
        figure
    } else {
        figure.child(caption(format!(
            "Empty the trash to get {} more back",
            util::human_bytes(trashed)
        )))
    }
}

/// The space a delete gives back right now: the marked copies, the same set
/// the Delete button counts. Unticking a copy lowers it; what the scan found
/// in all stays readable underneath while the two differ. When a scan has
/// just landed it counts up to its value once; a tick after that moves it
/// at once, never through a count.
fn reclaim_figure() -> View {
    let count = if first_sight(&COUNTED_GEN) {
        count_up(FIGURE_COUNT_MS)
    } else {
        signal(1.0)
    };
    answer_figure("Reclaimable", move || {
        let bytes = app().with(|s| actions::ticked(s).bytes);
        util::human_bytes((bytes as f64 * f64::from(count.get())).round() as u64)
    })
    // The line is always there, blank while everything found is marked: a
    // tick must not change the header's height and move the list under the
    // pointer.
    .child(caption(|| {
        app().with(|s| {
            if actions::ticked(s).bytes < s.reclaim_bytes {
                format!("of {} found", util::human_bytes(s.reclaim_bytes))
            } else {
                "\u{a0}".to_string()
            }
        })
    }))
    .tooltip(hint(
        "The space the ticked copies would give back if you deleted them now",
    ))
}

/// How long the Reclaimable figure counts up when a scan lands.
const FIGURE_COUNT_MS: f32 = 700.0;

/// The screen's answer, labelled above it: the largest number on the
/// screen, right-aligned with the toolbar's far edge.
fn answer_figure(label: &'static str, value: impl Fn() -> String + 'static) -> View {
    view()
        .flex_col()
        .items_end()
        .gap(px(space::XXS))
        .child(field_label(label))
        .child(figure(value))
}

/// The one line under the title: what the scan found, in the order a reader
/// acts on it — how many groups, and how many of their copies are marked to
/// delete. With no group left it says why the list is empty.
fn summary_line() -> Text {
    description(|| {
        app().with(|s| {
            if s.scanning {
                return "The list appears when the scan finishes.".to_string();
            }
            if s.groups.is_empty() {
                return match s.cleared {
                    Some(Cleared::Removed { .. }) => "Nothing left to review",
                    Some(Cleared::Restored { .. }) => "Nothing to review until the next scan",
                    None => "No duplicate groups",
                }
                .to_string();
            }
            format!(
                "{} · {} of {} marked to delete",
                counted(s.groups.len() as u64, "group", "groups"),
                actions::ticked(s).count,
                counted(s.dup_files, "duplicate", "duplicates"),
            )
        })
    })
}

/// The list's own controls, side by side at the toolbar's far end: show
/// every group, or fold them away. They are about reading the list, not
/// deciding anything, so they take the quietest look in the bar.
fn list_actions() -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::XXS))
        .child(icon_ghost_button(
            icons::EXPAND,
            "Expand all groups",
            || actions::set_all_expanded(true),
        ))
        .child(icon_ghost_button(
            icons::COLLAPSE,
            "Collapse all groups",
            || actions::set_all_expanded(false),
        ))
}

/// Two bands: what you do to the results, then what shapes them. Actions run
/// along the top — Back and Delete together on the left, the whole-set
/// commands on the right — and the options that only tune the list sit in a
/// second row, every one of them a segmented control of the same height.
fn action_row() -> View {
    view()
        .flex_col()
        .w_full()
        .gap(px(space::MD))
        .child(result_actions())
        .child(list_options())
}

/// The toolbar with no group on the page: only Back, since everything else
/// acts on groups.
fn back_only_row() -> View {
    view().w_full().child(action_bar(
        "Results actions",
        vec![action_group().child(back_button())],
    ))
}

/// The actions that act on the whole result set: destructive changes to the
/// files on the left, moving, reporting and folding on the right.
fn result_actions() -> impl IntoElement {
    action_bar(
        "Results actions",
        vec![
            action_group().child(back_button()).child(delete_bar()),
            action_group()
                .child(consolidate_button())
                .child(
                    icon_button(icons::EXPORT, "Export report…", actions::save_report)
                        .tooltip(hint("Export the duplicates as JSON, CSV or HTML")),
                )
                .child(list_actions()),
        ],
    )
}

/// How the list reads: its order and its layout. The rules that decide what
/// a group is and which copy it keeps belong to the scan, on New scan; here
/// they would re-pick every keeper and throw away the user's own ticks.
fn list_options() -> impl IntoElement {
    action_bar(
        "List options",
        vec![action_group().child(sort_toggle()).child(layout_toggle())],
    )
}

/// Re-running a finished scan. The cleanup workspace has no setup form and
/// no Back, so its header carries the next run itself. Hidden while a scan
/// runs, where the progress card's Stop is the only scan action.
fn scan_again_button() -> impl IntoElement {
    dynamic(|| {
        let scanning = computed(|| app().with(|s| s.scanning));
        when(!scanning.get(), || {
            secondary_button("Scan again", actions::start_scan)
        })
    })
}

/// Leave the results for the setup page. Hidden while a scan runs, where the
/// progress card's Stop owns cancelling.
fn back_button() -> impl IntoElement {
    when(!app().with(|s| s.scanning), || {
        icon_secondary_button(icons::ARROW_LEFT, "Back", actions::back_to_setup)
            .tooltip(hint("Back to scan setup"))
    })
}

/// One keeper rule at a time, as a segmented control.
fn keep_modes() -> View {
    view().child(dynamic(|| {
        // The rule alone rebuilds these segments; a scan tick would restart
        // their hover and drop a press mid-flight.
        let current = computed(|| app().with(|s| s.keep_index.unwrap_or(0)));
        segmented(KEEP_MODES, current.get(), actions::set_keep_rule)
    }))
}

/// The sort rule as two segments: path order, or the most space first.
fn sort_toggle() -> impl IntoElement {
    view().child(dynamic(|| {
        let biggest = computed(|| app().with(|s| s.sort_biggest));
        segmented(
            ["Path order", "Biggest first"],
            usize::from(biggest.get()),
            |index| {
                if app().with_untracked(|s| s.sort_biggest) != (index == 1) {
                    actions::toggle_sort_biggest();
                }
            },
        )
    }))
}

/// Grid or list: two-column preview cards, or one compact row list. It is a
/// structural choice, so switching rebuilds the list.
fn layout_toggle() -> View {
    view().child(dynamic(|| {
        let current = computed(|| app().with(|s| s.layout));
        let layouts = [Layout::Grid, Layout::List];
        let selected = layouts
            .iter()
            .position(|l| *l == current.get())
            .unwrap_or(0);
        icon_segmented(
            [(icons::GRID, "Grid layout"), (icons::LIST, "List layout")],
            selected,
            move |index| actions::set_layout(layouts[index]),
        )
    }))
}

/// The whole-set decision: how much is ticked, and the one button that acts
/// on it.
///
/// With nothing ticked it offers the missing step by name — `Select all
/// duplicates` — instead of rendering nothing and leaving the row with only
/// Back. Selection used to hide behind an unlabelled glyph in the header
/// pill, which is the one step between reading the list and deleting from it.
/// What the delete bar renders from, named field by field: the tuple it used
/// to be had grown to five positional values, and `(usize, u64, bool, bool,
/// usize)` does not say which `bool` is the trash setting.
#[derive(Clone, Copy, PartialEq, Eq)]
struct DeleteBar {
    count: usize,
    bytes: u64,
    can_undo: bool,
    trash: bool,
    removal: Removal,
    /// The ticked copies a hard link can replace, and their size: only those
    /// in byte-identical groups. Counted only while Hard link is chosen.
    linkable: (usize, u64),
    groups: usize,
}

fn delete_bar() -> View {
    view().child(dynamic(|| {
        // The selection and the trash switch decide this bar. A scan tick
        // rebuilding it replaced the Delete button under the pointer.
        let bar = computed(|| {
            app().with(|s| {
                let actions::Ticked { count, bytes } = actions::ticked(s);
                DeleteBar {
                    count,
                    bytes,
                    can_undo: !s.last_trashed.is_empty(),
                    trash: s.trash,
                    removal: s.removal,
                    linkable: if s.removal == Removal::Link {
                        let (links, _) = actions::link_plan(s);
                        (links.len(), links.iter().map(|l| l.size).sum())
                    } else {
                        (0, 0)
                    },
                    groups: s.groups.len(),
                }
            })
        });
        let DeleteBar {
            count,
            bytes,
            can_undo,
            trash,
            removal,
            linkable,
            groups,
        } = bar.get();
        let mut row = view().flex_row().gap(px(space::SM)).items_center();
        if count > 0 {
            // Trash and permanent deletes are different promises: a trashed
            // copy can come back, a permanent one cannot. The label carries
            // the difference, not just the tooltip.
            // The button names the act its confirmation was last set to, so
            // pressing it and reading the dialog make one promise.
            let unit = if count == 1 { "file" } else { "files" };
            let size = util::human_bytes(bytes);
            let button = match removal {
                Removal::Delete if trash => {
                    danger_button(format!("Delete {count} ({size})"), actions::request_delete)
                        .tooltip(hint(format!("Move {count} {unit} ({size}) to the trash")))
                }
                Removal::Delete => danger_button(
                    format!("Delete {count} permanently ({size})"),
                    actions::request_delete,
                )
                .tooltip(hint(format!(
                    "Permanently delete {count} {unit} ({size}). This can't be undone."
                ))),
                Removal::MoveAside => {
                    secondary_button(format!("Move {count} ({size})"), actions::request_delete)
                        .tooltip(hint(format!(
                            "Move {count} {unit} ({size}) into a folder you choose"
                        )))
                }
                // Only identical copies can be linked: the button counts
                // those, and with none it still opens the dialog, where
                // another way can be picked for the ticked copies.
                Removal::Link => match linkable {
                    (0, _) => secondary_button(
                        format!("Review {count} ticked…"),
                        actions::request_delete,
                    )
                    .tooltip(hint(
                        "None of the ticked copies is identical to its keeper, so none can become a hard link",
                    )),
                    (links, link_bytes) => {
                        let size = util::human_bytes(link_bytes);
                        let unit = if links == 1 { "file" } else { "files" };
                        danger_button(format!("Link {links} ({size})"), actions::request_delete)
                            .tooltip(hint(format!(
                                "Replace {links} {unit} with hard links to their keepers"
                            )))
                    }
                },
            };
            row = row
                .child(button)
                // Clearing is the undo of Select all, so it sits beside the
                // action it takes the target away from.
                .child(ghost_button("Clear selection", actions::select_none));
        } else if groups > 0 {
            row = row.child(
                icon_secondary_button(icons::SELECT_ALL, "Select all duplicates", || {
                    app().update(actions::reselect_all_dups)
                })
                .tooltip(hint("Tick every copy that is not a keeper")),
            );
        }
        if can_undo && actions::CAN_RESTORE {
            row = row.child(icon_secondary_button(
                icons::UNDO,
                "Restore from trash",
                actions::undo_delete,
            ));
        }
        row
    }))
}

/// The delete confirmation: a per-group dry run of exactly what will go,
/// with in-dialog Keep picks. Nothing is removed until "Delete N" is
/// pressed, and every file is re-hashed immediately before it is.
///
/// Reads its plan from the live selection, so ticking a box or changing a
/// keeper behind the dialog is reflected here instead of going stale.
/// The tallest the delete plan grows before it scrolls inside the dialog:
/// about three groups, so the dialog's buttons stay on screen at 640 pt.
const DELETE_PLAN_MAX_H: f32 = 320.0;

/// About how many characters of a path fit beside a copy's thumbnail and
/// its two keep controls in the delete dialog; the front is cut beyond it.
const PLAN_PATH_CHARS: usize = 30;

/// About how many characters of a folder fit under a going copy's name in
/// the delete dialog; the front is cut beyond it, so the distinguishing end
/// stays.
const PLAN_FOLDER_CHARS: usize = 22;

pub(crate) fn delete_confirm() -> View {
    let (count, bytes, groups, trash, removal, dest, link_count, link_bytes, unlinkable) = app()
        .with(|s| {
            let targets = actions::pending_targets(s);
            let (links, unlinkable) = actions::link_plan(s);
            (
                targets.len(),
                targets.iter().map(|t| t.size).sum::<u64>(),
                actions::confirm_groups(s),
                s.trash,
                s.removal,
                s.move_dest.clone(),
                links.len(),
                links.iter().map(|t| t.size).sum::<u64>(),
                unlinkable,
            )
        });
    // The plan fills the panel: without this the body is content-sized, so the
    // rules between groups stopped short of the panel's own edge.
    let mut body = view().flex_col().w_full().gap(px(space::MD));
    for (position, group) in groups.iter().enumerate() {
        let going = group.rows.len();
        // Each group reads as the pair it is: what stays, then what goes.
        // The old line ran both facts together ("Group #1 - keeps a - deletes
        // 1"), which is a sentence to parse at the moment of confirming a
        // deletion.
        let similarity = group
            .similarity
            .map(|s| format!(" · {:.0}% similar — not exact copies", s * 100.0))
            .unwrap_or_default();
        // A hard link needs identical bytes: a similar group is listed, and
        // says it is left alone, rather than vanishing from the plan.
        let fate = if removal == Removal::Link && !group.linkable {
            format!(
                "{} stay as they are: only identical copies can be linked",
                counted(going as u64, "copy", "copies")
            )
        } else {
            format!(
                "{} · {}{similarity}",
                counted(going as u64, "copy goes", "copies go"),
                util::human_bytes(group.bytes),
            )
        };
        let mut section = view()
            .flex_col()
            .gap(px(space::XS))
            .child(
                panel_title(format!("Keeping {}", group.keeper))
                    .flex_1()
                    .truncate(),
            )
            .child(caption(fate));
        for member in &group.rows {
            // The name leads, as on the review card: the folder under it is
            // the part that tells two copies of one name apart.
            let full = member.path.display().to_string();
            let file_name = member
                .path
                .file_name()
                .map_or_else(|| full.clone(), |n| n.to_string_lossy().into_owned());
            let folder = member
                .path
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            let cell = view()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                // `body` here is the dialog's list; the text role by path.
                .child(crate::ui::widgets::body(file_name).truncate())
                .child(caption(fit_text(&folder, PLAN_FOLDER_CHARS)).truncate())
                .tooltip(hint(full));
            let index = group.index;
            let keep_path = member.path.clone();
            let untick_path = member.path.clone();
            let poster = app().with_untracked(|s| s.posters.get(&member.path).cloned());
            let thumb = thumb_for(member, poster.as_deref());
            section = section.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap(px(space::SM))
                    .child(member_preview(&thumb, &member.path, PreviewSize::Thumb))
                    .child(cell)
                    .child(caption(util::human_bytes(member.size)))
                    .child(quiet_button("Keep this one instead", move || {
                        actions::set_keeper(index, &keep_path);
                    }))
                    .child(quiet_button("Keep too", move || {
                        actions::toggle_selected(&untick_path);
                    })),
            );
        }
        // One rule between plans: two groups of paths ran together as one
        // block, and the keeper line of the second looked like a repeat of
        // the first group's last row.
        if position > 0 {
            body = body.child(divider());
        }
        body = body.child(section);
    }
    if count == 0 {
        body = body.child(caption("Nothing is ticked."));
    }
    let files = counted(count as u64, "file", "files");
    let size = util::human_bytes(bytes);
    let checked = "every file is checked against the scan one last time first";
    let (heading, summary, commit) = match removal {
        Removal::Delete => {
            let any_similar = groups.iter().any(|g| g.similarity.is_some());
            let mut summary = if trash {
                format!(
                    "{files} ({size}) will move to the trash. You can put them back from the trash until it is emptied. Keepers stay, and {checked}."
                )
            } else {
                format!(
                    "{files} ({size}) will be deleted permanently. This can't be undone. Keepers stay, and {checked}."
                )
            };
            if any_similar {
                summary.push_str(" Compare the marked copies before you delete: similar files are matched by sight, not byte for byte.");
            }
            let (heading, label) = if trash {
                (
                    format!("Move {files} to the trash?"),
                    format!("Move {count} to the trash"),
                )
            } else {
                (
                    format!("Delete {files} permanently?"),
                    format!("Delete {count} permanently"),
                )
            };
            let commit = (count > 0).then(|| danger_button(label, actions::confirm_delete));
            (heading, summary, commit)
        }
        Removal::MoveAside => {
            // The folder line under the choice names the folder; the
            // summary only says what a move does there.
            let summary = format!(
                "{files} ({size}) move into the folder below; a name already taken there gets a number. Nothing is deleted, and {checked}."
            );
            let label = format!("Move {count}");
            let commit = (count > 0 && dest.is_some())
                .then(|| primary_button(label, actions::confirm_delete));
            (format!("Move {files} to a folder?"), summary, commit)
        }
        Removal::Link => {
            let linked = counted(link_count as u64, "file", "files");
            let mut summary = format!(
                "Each becomes a hard link to its group's keeper: the paths stay and {} comes back. Editing one linked path then changes all of them. {}; a copy on another drive than its keeper stays as it is.",
                util::human_bytes(link_bytes),
                capitalized(checked),
            );
            if unlinkable > 0 {
                summary.push_str(&format!(
                    " {} in similar groups {} not identical, so {} as {} {}.",
                    counted(unlinkable as u64, "copy", "copies"),
                    if unlinkable == 1 { "is" } else { "are" },
                    if unlinkable == 1 {
                        "it stays"
                    } else {
                        "they stay"
                    },
                    if unlinkable == 1 { "it" } else { "they" },
                    if unlinkable == 1 { "is" } else { "are" },
                ));
            }
            let label = format!("Link {link_count}");
            let commit = (link_count > 0).then(|| danger_button(label, actions::confirm_delete));
            (
                format!("Replace {linked} with hard links?"),
                summary,
                commit,
            )
        }
    };
    // What happens to the ticked copies is decided here, beside the plan it
    // applies to; the toolbar button then names the same act.
    let mut content = view().flex_col().w_full().gap(px(space::MD)).child(
        view()
            .flex_row()
            .flex_wrap()
            .items_center()
            .gap(px(space::SM))
            .child(field_label("Ticked copies"))
            .child(radio_segmented(
                REMOVAL_LABELS,
                removal.index(),
                actions::set_removal,
            )),
    );
    if removal == Removal::MoveAside {
        let (shown, full, button) = match &dest {
            Some(dest) => {
                let full = dest.display().to_string();
                (
                    fit_text(&full, PLAN_PATH_CHARS),
                    Some(full),
                    "Change folder…",
                )
            }
            None => ("No folder chosen yet".to_string(), None, "Choose folder…"),
        };
        let line = caption(shown).flex_1().truncate();
        let line = match full {
            Some(full) => line.tooltip(hint(full)),
            None => line,
        };
        content = content.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::SM))
                .child(line)
                .child(secondary_button(button, actions::pick_move_dir)),
        );
    }
    content = content.child(fast_scroll_max(&DELETE_AT, px(DELETE_PLAN_MAX_H), body));
    let mut buttons = view().child(secondary_button("Cancel", actions::cancel_delete));
    if let Some(commit) = commit {
        buttons = buttons.child(commit);
    }
    alert_dialog(heading, summary, content, buttons, actions::cancel_delete)
}

/// `text` with its first letter in capitals, for a clause that starts a
/// sentence in one summary and ends one in another.
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// The three ways the delete confirmation can deal with the ticked copies,
/// in [`Removal::ALL`] order.
const REMOVAL_LABELS: [&str; 3] = ["Delete", "Move to folder", "Hard link"];

/// Consolidate: gather one copy of every group into a folder the user picks.
/// It moves files, so it is named on its button, and the confirmation opens
/// first ([`consolidate_confirm`]); the folder is picked inside it.
fn consolidate_button() -> View {
    icon_secondary_button(icons::FOLDER_MOVE, "Consolidate", actions::open_consolidate)
        .tooltip(hint("Move one copy of each group into a folder you choose"))
}

/// Moves drawn in the consolidate confirmation before it summarises the rest.
const CONSOLIDATE_ROWS: usize = 8;

/// The consolidate confirmation: every keeper that will move, where it comes
/// from and the name it arrives under when that one is taken. Nothing moves
/// until "Move N" is pressed. Before a folder is picked it asks for one; the
/// plan is read when the dialog opens: the list behind a modal cannot change.
pub(crate) fn consolidate_confirm() -> View {
    let picked = app().with_untracked(|s| {
        s.consolidate_dest.clone().map(|dest| {
            let (moves, skipped) = crate::actions::plan_consolidation(&s.groups, &dest);
            (dest, moves, skipped.len())
        })
    });
    let Some((dest, moves, skipped)) = picked else {
        // No folder yet: the dialog asks for one and explains what follows.
        // The space the plan would hold is shown, so the ask is not blind.
        let bytes: u64 = app().with_untracked(|s| {
            s.groups
                .iter()
                .flat_map(|g| g.members.iter())
                .filter(|m| m.keep)
                .map(|m| m.size)
                .sum()
        });
        return alert_dialog(
            "Consolidate keepers?".to_string(),
            format!(
                "One copy of every group moves into a folder you pick ({} so far). Nothing moves until you have picked the folder and confirmed the list.",
                util::human_bytes(bytes),
            ),
            view().child(secondary_button(
                "Choose a folder…",
                actions::pick_consolidate_dir,
            )),
            view().child(secondary_button("Cancel", actions::cancel_consolidate)),
            actions::cancel_consolidate,
        );
    };
    let count = moves.len();
    let bytes: u64 = moves.iter().map(|m| m.target.size).sum();
    let mut body = view().flex_col().w_full().gap(px(space::MD));
    for planned in moves.iter().take(CONSOLIDATE_ROWS) {
        let target = &planned.target;
        let file_name = |p: &Path| {
            p.file_name().map_or_else(
                || p.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            )
        };
        let from = target
            .from
            .parent()
            .map_or_else(String::new, |p| p.display().to_string());
        let shown = fit_text(&from, PLAN_PATH_CHARS);
        let mut row = view()
            .flex_col()
            .gap(px(space::XXS))
            .child(panel_title(file_name(&target.from)).truncate())
            .child(caption(format!("from {shown}")).truncate());
        if target.from.file_name() != target.to.file_name() {
            row = row.child(caption(format!(
                "arrives as {}: that name is taken",
                file_name(&target.to)
            )));
        }
        body = body.child(row);
    }
    if count > CONSOLIDATE_ROWS {
        body = body.child(caption(format!(
            "... and {} more",
            counted((count - CONSOLIDATE_ROWS) as u64, "keeper", "keepers")
        )));
    }
    let mut summary = format!(
        "{} ({}) move into {}. Each is checked against the scan first; a file that changed stays where it is.",
        counted(count as u64, "keeper", "keepers"),
        util::human_bytes(bytes),
        fit_text(&dest.display().to_string(), PLAN_PATH_CHARS),
    );
    if skipped > 0 {
        summary.push_str(&format!(
            " {} protected or already there, so {} not move.",
            counted(skipped as u64, "keeper is", "keepers are"),
            if skipped == 1 { "it does" } else { "they do" },
        ));
    }
    alert_dialog(
        format!(
            "Consolidate {}?",
            counted(count as u64, "keeper", "keepers")
        ),
        summary,
        fast_scroll_max(&CONSOLIDATE_AT, px(DELETE_PLAN_MAX_H), body),
        view()
            .child(secondary_button("Cancel", actions::cancel_consolidate))
            .child(primary_button(
                format!("Move {}", counted(count as u64, "keeper", "keepers")),
                actions::confirm_consolidate,
            )),
        actions::cancel_consolidate,
    )
}

/// What decides when the results list is rebuilt. It leaves keeper picks and
/// ticks out on purpose: those are member-level decisions ([`member_slot`])
/// and must not rebuild, or reorder, the list.
type ListSignature = (u64, bool, bool, usize, usize, bool);

fn list_body((_, scanning, has_scanned, total, limit, ffprobe_note): ListSignature) -> View {
    // The column owns the pane's width. Without it the list is content-sized,
    // so a wide row (the setup form's controls) stretches the whole column past
    // the window instead of wrapping inside it.
    let mut list = view().flex_col().w_full().min_w(px(0.0)).gap(px(space::MD));
    if scanning {
        list = list.child(scan_card());
    }
    if !has_scanned && !scanning {
        list = list.child(dynamic(setup));
    }
    if ffprobe_note {
        list = list.child(note_card(
            "ffprobe not found on PATH - install FFmpeg for media resolution/duration.",
        ));
    }
    if has_scanned && total == 0 {
        let cleared = app().with_untracked(|s| s.cleared);
        list = list.child(match cleared {
            Some(cleared) => cleared_card(cleared),
            None => nothing_found_card(
                "No duplicates found",
                "Every file is unique at this similarity. Add another folder or loosen the threshold, then scan again.",
                true,
            ),
        });
    }
    // Structural snapshot, read untracked: nothing here subscribes, so this
    // list only re-runs when the signature above changes.
    let (groups, layout) = app().with_untracked(|s| (s.groups.clone(), s.layout));
    // A scan's own landing deals its cards in one after another; a page
    // coming back to results it already showed does not.
    let landed = has_scanned && !scanning && total > 0 && first_sight(&ARRIVED_GEN);
    for (position, group) in groups.iter().take(limit).enumerate() {
        let slot = group_slot(position, group, layout);
        list = list.child(if landed {
            let arrive = arriving(position);
            slot.bind_motion(&arrive)
        } else {
            slot
        });
    }
    if total > limit {
        let remaining = total - limit;
        list = list.child(view().flex_row().justify_center().child(ghost_button(
            format!("Show more ({remaining} remaining)"),
            actions::show_more_groups,
        )));
    }
    if landed {
        return list;
    }
    // Any other rebuild — a new filter, a delete, Show more — fades in where
    // it stands. It fades rather than rises because it keeps the scroll's
    // own place; only the page itself travels.
    let fade = fading_in();
    list.bind_motion(&fade)
}

thread_local! {
    /// The scan whose cards last arrived, and whose figure last counted up.
    static ARRIVED_GEN: Cell<u64> = const { Cell::new(0) };
    static COUNTED_GEN: Cell<u64> = const { Cell::new(0) };
}

/// Whether `seen` meets the current scan's results for the first time: the
/// scan has just landed, rather than a page coming back to results it
/// already showed.
fn first_sight(seen: &'static std::thread::LocalKey<Cell<u64>>) -> bool {
    let generation = app().with_untracked(|s| s.scan_gen);
    seen.with(|cell| cell.replace(generation) != generation)
}

/// The running scan's phase, its count and Stop. Progress has a signal of
/// its own: reading it here subscribes this row alone, so a tick never
/// rebuilds the rest of the window.
fn progress_header() -> View {
    let (done, total, determinate, phase) =
        scan_status().with(|p| (p.done, p.total, p.determinate, p.phase.clone()));
    // A scan counts twice — files to hash, then media to compare — so the
    // bar legitimately restarts with a fresh total when the phase changes.
    // The phase is the heading and the counts are the caption, so that
    // restart reads as a new stage instead of the same bar going backwards.
    // A phase that cannot count has no caption: it would only repeat the
    // heading.
    let mut words = view()
        .flex_col()
        .min_w(px(0.0))
        .gap(px(space::XXS))
        .child(panel_title(phase));
    if determinate && total > 0 {
        words = words.child(caption(format!("{done} of {total}")).tabular_nums());
    }
    view()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(space::MD))
        .child(words)
        .child(danger_button("Stop", actions::stop_scan))
}

/// A running scan: the ensō turning beside its progress. The ensō and the
/// bar live outside the part a tick rebuilds, so the ensō's turn never
/// restarts, and the bar rebuilds only when its fill moves — while the scan
/// cannot count, its sweep runs on undisturbed.
fn scan_card() -> View {
    let fill = computed(|| {
        scan_status().with(|p| {
            (p.determinate && p.total > 0).then(|| (p.done.min(p.total) * 1000 / p.total) as u32)
        })
    });
    card()
        .flex_row()
        .items_center()
        .gap(px(space::LG))
        .p(px(space::LG))
        .child(turning_enso(ENSO_SIDE))
        .child(
            view()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(space::MD))
                .child(dynamic(progress_header))
                .child(dynamic(move || match fill.get() {
                    Some(permille) => progress_bar(permille as f32 / 1000.0, true),
                    None => progress_bar(0.0, false),
                })),
        )
}

/// The ensō's side on the scan card: the height of the phase and its count.
const ENSO_SIDE: f32 = 36.0;

/// The setup page: what the next scan covers and how it decides, as one
/// form, with the scan itself as the screen's one primary action. Its title
/// row sits where the results' title will, so starting a scan changes the
/// content under a header that stays put.
fn setup() -> View {
    // The page arrives as one piece instead of appearing fully formed.
    let enter = entering();
    view()
        .flex_col()
        .w_full()
        .gap(px(space::LG))
        .child(setup_header())
        .child(setup_form())
        .bind_motion(&enter)
}

/// The title, the scope of the next run in one plain line, and Scan.
fn setup_header() -> View {
    view()
        .flex_row()
        .flex_wrap()
        .items_end()
        .justify_between()
        .gap(px(space::LG))
        .child(title_block(heading("New scan"), description(scope_line)))
        .child(primary_button(scan_label, actions::start_scan))
}

/// What the next run will do, from the choices below it: how many folders,
/// what it looks for, and where a delete sends the copies. Nothing is
/// removed by a scan, and the line says so where the reader decides.
fn scope_line() -> String {
    let (folders, trash, removal, find_index) =
        app().with(|s| (s.folders.len(), s.trash, s.removal, s.find_index));
    if folders == 0 {
        return "Add a folder below or drop one on the window. A scan only reads; nothing is deleted until you confirm.".to_string();
    }
    let mode = crate::ui_helpers::find_mode(find_index);
    let outcome = if mode != crate::cli::FindMode::Duplicates {
        match actions::findings_act(mode) {
            actions::FindingsAct::ReportOnly => "findings are listed, nothing is deleted",
            actions::FindingsAct::StripMetadata => {
                "photos are listed; their metadata goes only after you confirm"
            }
            actions::FindingsAct::Delete if trash => {
                "findings are listed; they go to the trash only after you confirm"
            }
            actions::FindingsAct::Delete => {
                "findings are listed; they are deleted only after you confirm"
            }
        }
    } else {
        match removal {
            Removal::Delete if trash => "copies go to the trash after you confirm",
            Removal::Delete => "copies are deleted permanently after you confirm",
            Removal::MoveAside => "copies move into a folder after you confirm",
            Removal::Link => "copies become hard links after you confirm",
        }
    };
    format!(
        "{} in {} · {outcome}",
        find_label(),
        counted(folders as u64, "folder", "folders"),
    )
}

/// Whether the next scan looks for duplicates, the scan whose form carries
/// the keeper, similarity and video rows.
fn finds_duplicates() -> bool {
    crate::ui_helpers::find_mode(app().with(|s| s.find_index)) == crate::cli::FindMode::Duplicates
}

/// The scan's choices as labelled rows on one card, every rule visible:
/// where, what, which copy stays, how close is close, where deletes go, and
/// what video support the machine has.
fn setup_form() -> View {
    view().w_full().child(dynamic(|| {
        // The scan type alone reshapes the form: the keeper, similarity,
        // delete and video rows only mean something to a duplicate scan.
        let shape = computed(|| {
            let mode = crate::ui_helpers::find_mode(app().with(|s| s.find_index));
            (
                mode == crate::cli::FindMode::Duplicates,
                actions::findings_act(mode) == actions::FindingsAct::Delete,
            )
        });
        let (duplicates, deletes) = shape.get();
        setup_rows(duplicates, deletes)
    }))
}

/// The form's rows for one scan type. `deletes` is a cleanup scan whose
/// findings can be deleted, which makes where a delete goes worth a row.
fn setup_rows(duplicates: bool, deletes: bool) -> View {
    let mut rows: Vec<View> = vec![
        form_row(
            "Folders",
            Some(
                "Type a path and press Add folder, browse for one, or drop folders on the window.",
            ),
            view()
                .flex_row()
                .items_center()
                .gap(px(space::SM))
                .w_full()
                .child(field(
                    "Scan a folder",
                    || app().with(|s| s.folder_input.clone()),
                    |value| app().update(|s| s.folder_input = value),
                ))
                .child(icon_secondary_button(
                    icons::PLUS,
                    "Add folder",
                    actions::add_from_input,
                ))
                // The native folder picker, for a reader who would rather
                // pick than type; the same action as the sidebar's Browse.
                .child(icon_secondary_button(
                    icons::FOLDER,
                    "Browse",
                    actions::browse_folder,
                )),
        ),
        form_row(
            "Look for",
            None,
            view()
                .flex_col()
                .gap(px(space::XS))
                .child(find_select())
                .child(caption(|| {
                    crate::ui_helpers::find_mode_caption(app().with(|s| s.find_index))
                })),
        ),
    ];
    if duplicates {
        rows.extend([
        form_row(
            "Keep",
            Some(
                "The copy of each group that survives a delete. First is path order; Best is the highest resolution.",
            ),
            keep_modes(),
        ),
        form_row(
            "Similarity",
            Some(
                "How close two photos or videos must be to count as the same. 97% catches re-saves and re-encodes.",
            ),
            view().child(sim_pill()),
        ),
        form_row("Deleted copies", None, view().child(trash_select())),
        form_row("Video", None, media_status()),
        ]);
    } else if deletes {
        rows.push(form_row(
            "Deleted items",
            None,
            view().child(trash_select()),
        ));
    }
    let mut form = card().w_full();
    for (index, row) in rows.into_iter().enumerate() {
        if index > 0 {
            form = form.child(view().px(px(space::LG)).child(divider()));
        }
        form = form.child(row);
    }
    form
}

/// The label column of the setup form: wide enough for its longest label,
/// so every control starts on one line down the card.
const FORM_LABEL_W: f32 = 148.0;

/// One row of the setup form: the label on the left, the control and its
/// explanation on the right, both on the card's own inset.
fn form_row(label: &'static str, note: Option<&'static str>, control: View) -> View {
    // Controls keep their own width (a segmented control stretched to the
    // card's edge reads as a progress bar); the folder row asks for the full
    // width itself.
    let mut right = view()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .items_start()
        .gap(px(space::XS))
        .child(control);
    if let Some(note) = note {
        right = right.child(caption(note));
    }
    view()
        .flex_row()
        .items_start()
        .gap(px(space::LG))
        .px(px(space::LG))
        // 10 pt: the whole form, every row with its note, fits the default
        // 1120×640 window without scrolling.
        .py(px(space::SM + space::XXS))
        .child(
            view()
                .w(px(FORM_LABEL_W))
                .flex_shrink_0()
                .min_h(px(size::CONTROL))
                .justify_center()
                .child(panel_title(label)),
        )
        .child(right)
}

/// What the next scan looks for, as one choice out of the scan modes.
fn find_select() -> impl IntoElement {
    let items: Vec<SelectItem> = crate::ui::settings::FIND_LABELS
        .iter()
        .map(|label| SelectItem::new(*label, *label))
        .collect();
    option_select(
        items,
        "What to find",
        || Some(find_label()),
        |picked: &Option<SharedString>| {
            let Some(picked) = picked.as_deref() else {
                return;
            };
            if let Some(index) = crate::ui::settings::FIND_LABELS
                .iter()
                .position(|label| *label == picked)
            {
                app().update(|s| s.find_index = Some(index));
                actions::save_prefs();
            }
        },
        dropdown_trigger(icons::SEARCH, find_label),
    )
}

/// Where a delete puts the files, as one choice of two.
fn trash_select() -> impl IntoElement {
    option_select(
        vec![
            SelectItem::new("trash", "Move to trash"),
            SelectItem::new("delete", "Delete permanently"),
        ],
        "Where deleted files go",
        || {
            Some(SharedString::from(if app().with(|s| s.trash) {
                "trash"
            } else {
                "delete"
            }))
        },
        |picked: &Option<SharedString>| {
            app().update(|s| s.trash = matches!(picked.as_deref(), Some("trash")));
            actions::save_prefs();
        },
        dropdown_trigger(icons::TRASH, || {
            SharedString::from(trash_label(app().with(|s| s.trash)))
        }),
    )
}

fn find_label() -> SharedString {
    let index = app().with(|s| s.find_index.unwrap_or(0));
    SharedString::from(
        crate::ui::settings::FIND_LABELS[index.min(crate::ui::settings::FIND_LABELS.len() - 1)],
    )
}

fn trash_label(trash: bool) -> &'static str {
    if trash {
        "Move to trash"
    } else {
        "Delete permanently"
    }
}

/// The similarity floor of a run: one value out of the five offered, as a
/// dropdown like the other choices on the setup form.
fn sim_pill() -> impl IntoElement {
    let items: Vec<SelectItem> = SIM_STEPS
        .iter()
        .map(|step| SelectItem::new(step.to_string(), format!("{step}%")))
        .collect();
    option_select(
        items,
        "Similarity floor",
        || {
            Some(SharedString::from(actions::format_similarity_pct(
                current_similarity(),
            )))
        },
        |picked: &Option<SharedString>| {
            let Some(picked) = picked else {
                return;
            };
            let text = picked.to_string();
            app().update(|s| s.min_sim_input = text.clone());
            actions::apply_sim_input(&text);
        },
        dropdown_trigger(icons::SLIDERS, || {
            SharedString::from(format!(
                "{}%",
                actions::format_similarity_pct(current_similarity())
            ))
        }),
    )
}

/// The similarity steps the setup form offers, loosest last.
const SIM_STEPS: [f64; 5] = [100.0, 97.0, 95.0, 90.0, 85.0];

fn current_similarity() -> f64 {
    app()
        .with(|s| s.min_sim_input.trim().parse::<f64>().ok())
        .unwrap_or(97.0)
}

/// The scan button's label, per scan type. Read inside the setup page's
/// dynamic region, so changing what to look for relabels it without a
/// rebuild.
fn scan_label() -> &'static str {
    if finds_duplicates() {
        "Scan for duplicates"
    } else {
        "Run cleanup scan"
    }
}

/// What the media chip says, from the two probe answers.
///
/// Previews need ffmpeg alone: it cuts the posters and plays the clip.
/// ffprobe only adds the resolution and duration that video matching uses, so
/// calling its absence "no previews" denied the thumbnails that still work.
#[must_use]
pub(crate) fn media_status_text(
    checked: bool,
    ffmpeg: bool,
    ffprobe: bool,
) -> (&'static str, &'static str) {
    if !checked {
        ("Checking video support…", "Looking for FFmpeg on PATH.")
    } else if ffmpeg && ffprobe {
        (
            "Video previews on",
            "FFmpeg adds video previews; ffprobe adds resolution, duration and video matching.",
        )
    } else if ffmpeg {
        (
            "Video previews on",
            "Thumbnails and playback work. Install ffprobe to add resolution, duration and video matching.",
        )
    } else {
        (
            "No video previews",
            "FFmpeg adds video previews and video matching. Image comparison works without it.",
        )
    }
}

/// ffmpeg/ffprobe availability as a row of the setup form: what the reader
/// gets, then what adds the rest. The detail is on the page, not in a
/// tooltip — it decides whether video matching runs at all.
fn media_status() -> View {
    view().child(dynamic(|| {
        let probe = computed(|| app().with(|s| (s.media_checked, s.ffmpeg_ok, s.ffprobe_ok)));
        let (checked, ffmpeg, ffprobe) = probe.get();
        let (label, detail) = media_status_text(checked, ffmpeg, ffprobe);
        view()
            .flex_col()
            .gap(px(space::XS))
            .child(
                view()
                    .min_h(px(size::CONTROL))
                    .justify_center()
                    .child(panel_title(label)),
            )
            .child(caption(detail))
    }))
}

/// Every group is gone because the user cleared them: what went, where to,
/// and that the keepers are untouched. It is the end of the task, so it reads
/// as an outcome, never as a scan that found nothing. After a trash delete it
/// offers the trash's way back while this platform can restore and the last
/// delete has not been undone yet.
fn cleared_card(cleared: Cleared) -> View {
    let (title, detail) = match cleared {
        Cleared::Restored { files } => (
            format!("{} restored", counted(files, "copy", "copies")),
            "They are back where they were. Scan again to review them.".to_string(),
        ),
        Cleared::Removed {
            files,
            trashed,
            freed,
            moved,
        } => {
            let copies = counted(files, "copy", "copies");
            let title = if freed == 0 && trashed == 0 {
                format!("{copies} moved aside")
            } else if freed == 0 && moved == 0 {
                format!("{copies} moved to the trash")
            } else if trashed == 0 && moved == 0 {
                format!("{copies} removed")
            } else {
                format!("{copies} cleared")
            };
            // One sentence per place the copies went, in the order the space
            // comes back: free now, free once the trash is emptied, not freed.
            let mut detail = "Your keepers stay where they were.".to_string();
            if freed > 0 && (trashed > 0 || moved > 0) {
                detail.push_str(&format!(" {} freed.", util::human_bytes(freed)));
            }
            if trashed > 0 {
                detail.push_str(&format!(
                    " {} waits in the trash until you empty it.",
                    util::human_bytes(trashed)
                ));
            }
            if moved > 0 {
                detail.push_str(&format!(
                    " {} moved into the folder you chose.",
                    util::human_bytes(moved)
                ));
            }
            (title, detail)
        }
    };
    let mut buttons = view()
        .flex_row()
        .gap(px(space::SM))
        .child(primary_button("Scan again", actions::start_scan));
    let can_restore = matches!(cleared, Cleared::Removed { trashed, .. } if trashed > 0)
        && actions::CAN_RESTORE
        && app().with_untracked(|s| !s.last_trashed.is_empty());
    if can_restore {
        buttons = buttons.child(icon_secondary_button(
            icons::UNDO,
            "Restore from trash",
            actions::undo_delete,
        ));
    }
    // The clean-up is settled: the seal says so in the margin of the card.
    // Copies that came back from the trash are not a settled clean-up.
    let settled = matches!(cleared, Cleared::Removed { .. });
    outcome_card(title, detail, settled).child(buttons)
}

/// The card a finished act leaves: its title over what happened, and for a
/// clean-up that went through, the 済 (settled) seal pressed beside them.
fn outcome_card(title: String, detail: String, settled: bool) -> View {
    card().gap(px(space::LG)).p(px(space::XL)).child(
        view()
            .flex_row()
            .items_start()
            .gap(px(space::XL))
            .child(
                view()
                    .flex_col()
                    .flex_1()
                    .min_w(px(0.0))
                    .gap(px(space::XS))
                    .child(section_title(title))
                    .child(description(detail)),
            )
            .child(when(settled, || pressed_seal(SEAL_SIDE))),
    )
}

/// The settled seal's side: it reads as a seal across the card's two lines
/// of text, not as an icon.
const SEAL_SIDE: f32 = 44.0;

/// A scan that found nothing: what to change and where. `rescan` adds the
/// button that runs the same scan again — on the duplicates page the scan form
/// lives back on the setup page, so without it this state is a dead end. The
/// cleanup workspace passes `false`: its header already carries the rerun.
fn nothing_found_card(message: &'static str, hint: &'static str, rescan: bool) -> View {
    let mut card = card().gap(px(space::MD)).p(px(space::XL)).child(
        view()
            .flex_col()
            .gap(px(space::XS))
            .child(section_title(message))
            .child(description(hint)),
    );
    if rescan {
        card = card.child(
            view()
                .flex_row()
                .child(primary_button("Scan again", actions::start_scan)),
        );
    }
    card
}

/// Every finding is gone because the user acted on them: what went and where,
/// so the empty list reads as the end of the task, not as a clean scan. After
/// a trashed delete it offers the trash's way back where the OS can restore.
fn findings_cleared_card(done: FindingsDone) -> View {
    let bytes = util::human_bytes(done.bytes);
    let (title, detail) = match done.went {
        FindingsWent::Trash => (
            format!(
                "{} moved to the trash",
                counted(done.items, "item", "items")
            ),
            format!("{bytes} waits in the trash until you empty it."),
        ),
        FindingsWent::Deleted => (
            format!("{} deleted", counted(done.items, "item", "items")),
            format!("{bytes} freed."),
        ),
        FindingsWent::Restored => (
            format!("{} restored", counted(done.items, "item", "items")),
            "They are back where they were. Scan again to review them.".to_string(),
        ),
        FindingsWent::Stripped => (
            format!(
                "Metadata removed from {}",
                counted(done.items, "photo", "photos")
            ),
            format!("{bytes} freed. The photos themselves stay where they were."),
        ),
    };
    let settled = done.went != FindingsWent::Restored;
    let mut card = outcome_card(title, detail, settled);
    let can_restore = done.went == FindingsWent::Trash
        && actions::CAN_RESTORE
        && app().with_untracked(|s| !s.last_trashed.is_empty());
    if can_restore {
        card = card.child(view().flex_row().child(icon_secondary_button(
            icons::UNDO,
            "Restore from trash",
            actions::undo_delete,
        )));
    }
    card
}

fn note_card(message: &'static str) -> View {
    notice().child(caption(message))
}

/// One group card, as the wheel's step: the layout decides a card's height,
/// and a group whose members have no preview stays a compact card even in
/// grid layout.
fn card_step() -> f32 {
    app().with_untracked(|s| {
        let grid = s.layout == Layout::Grid
            && s.groups.first().is_some_and(|g| {
                g.members.len() >= 2
                    && g.members
                        .iter()
                        .all(|m| is_grid_preview(&thumb_for(m, None)))
            });
        if grid { CARD_STEP_GRID } else { CARD_STEP_LIST }
    })
}

/// The group's own line: how many copies, how big each one is, and how close
/// they are when the group came from the similarity pass. No group number —
/// the reader acts on files, not on row ids.
fn group_header(group: &Group) -> String {
    let copies = group.members.len();
    let first = group.members.first().map(|m| m.size).unwrap_or(0);
    let size = if group.members.iter().all(|m| m.size == first) {
        format!(" · {} each", util::human_bytes(first))
    } else {
        String::new()
    };
    let similarity = group
        .similarity
        .map(|s| format!(" · {:.0}% similar", s * 100.0))
        .unwrap_or_default();
    format!(
        "{}{size}{similarity}",
        counted(copies as u64, "copy", "copies")
    )
}

/// One group's slot: rebuilt only when its disclosure flips. Keeper picks,
/// ticks and previews are member decisions and update their own cards.
///
/// Open is the default — `collapsed` holds only what the user closed — so a
/// freshly scanned group renders expanded without any per-group bookkeeping.
fn group_slot(position: usize, group: &Group, layout: Layout) -> View {
    let group = group.clone();
    let expanded = computed({
        let index = group.index;
        move || app().with(|s| !s.collapsed.contains(&index))
    });
    view().child(dynamic(move || {
        group_card(position, &group, expanded.get(), layout)
    }))
}

fn group_card(position: usize, group: &Group, expanded: bool, layout: Layout) -> View {
    let index = group.index;
    let header = group_header(group);
    let members = group.members.clone();
    // Grid is the original layout: media groups whose every member has a
    // preview become two-column cards with a large preview, and anything else
    // (plain files, archives, a group with a copy whose frame cannot be
    // decoded) stays a compact list. List draws every group as rows, so an
    // image and an archive always occupy the same box.
    let grid = layout == Layout::Grid
        && members.len() >= 2
        && members.iter().all(|m| is_grid_preview(&thumb_for(m, None)));
    // The disclosure insets the body onto the header's text column.
    let mut body = view().flex_col().gap(px(space::SM));
    if grid {
        for (row_index, pair) in members.chunks(2).enumerate() {
            let mut row = view().flex_row().items_stretch().gap(px(space::SM));
            for (column, member) in pair.iter().enumerate() {
                row = row.child(
                    // The row's own child, not the card, is what flex lays
                    // out: give the slot the equal share so both cards — and
                    // both previews — come out the same width.
                    view()
                        .flex_1()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .child(member_slot(
                            position,
                            index,
                            row_index * 2 + column,
                            member.clone(),
                            true,
                        )),
                );
            }
            if pair.len() == 1 {
                // Odd tail: leave the lone card at half width.
                row = row.child(view().flex_1().min_w(px(0.0)));
            }
            body = body.child(row);
        }
    } else {
        for (member_pos, member) in members.iter().enumerate() {
            body = body.child(member_slot(
                position,
                index,
                member_pos,
                member.clone(),
                false,
            ));
        }
    }
    // The header counts follow the ticks, not just the keepers: a group with
    // nothing selected is not "1 copy to delete", and the reclaimable figure
    // is what the ticked copies would actually give back. They update on
    // their own instead of taking the group's cards with them.
    let stats = computed(move || {
        app().with(|s| {
            s.groups
                .iter()
                .find(|g| g.index == index)
                .map_or((1, 0, 0), |g| {
                    let mut copies = 0usize;
                    let mut bytes = 0u64;
                    for m in &g.members {
                        if m.deletable() && s.selected.contains(&m.path) {
                            copies += 1;
                            bytes += m.size;
                        }
                    }
                    (g.keep_count(), copies, bytes)
                })
        })
    });
    let tag = members.first().map_or(Kind::File, |m| file_kind(&m.path));
    let header = view()
        .flex_row()
        .flex_1()
        .min_w(px(0.0))
        .items_center()
        .gap(px(space::MD))
        .child(badge(kind_label(tag), BadgeTone::Kind(tag)))
        .child(name(header).flex_1().truncate())
        .child(dynamic(move || {
            let (kept, copies, bytes) = stats.get();
            let mut row = view()
                .flex_row()
                .items_center()
                .gap(px(space::MD))
                .child(caption(if copies == 0 {
                    // Nothing ticked is a state worth naming: the group is
                    // intact, not half-deleted.
                    format!("{kept} kept · nothing ticked")
                } else {
                    format!(
                        "{kept} kept · {} to delete",
                        counted(copies as u64, "copy", "copies"),
                    )
                }));
            if copies > 0 {
                // The group's share of the answer figure, in ink: the hue
                // belongs to the total alone.
                row = row.child(amount(format!("{} to free", util::human_bytes(bytes))));
            }
            row
        }));
    disclosure(
        expanded,
        move || actions::toggle_expanded(index),
        header,
        body,
    )
}

/// File name for a member, or the whole path when the file has none.
fn member_name(member: &GroupMember) -> String {
    member.path.file_name().map_or_else(
        || member.path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Folder the member lives in: the dimmed second line of either layout.
fn member_parent(member: &GroupMember) -> String {
    member
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default()
}

/// One member's slot, with its own rebuild signature: its keeper flag (and,
/// on the keeper, the group's keep reason), its tick and its poster frame.
/// Pressing Keep on another member therefore updates the two cards
/// involved and nothing else — the group keeps its order, its scroll and
/// its other cards untouched.
fn member_slot(
    group_pos: usize,
    group_index: usize,
    member_pos: usize,
    member: GroupMember,
    grid: bool,
) -> View {
    let path = member.path.clone();
    let signature = computed(move || {
        app().with(|s| {
            let group = s.groups.get(group_pos);
            let current = group.and_then(|g| g.members.get(member_pos));
            (
                current.is_some_and(|m| m.keep),
                group.zip(current).and_then(|(g, m)| g.reason_for(m)),
                s.selected.contains(&path),
                s.posters.get(&path).cloned(),
                s.playing.as_deref() == Some(path.as_path()),
            )
        })
    });
    // The decision this slot last showed: a change of it (and only that, not
    // a poster landing or a clip playing) soaks the new wash in.
    let shown: Rc<Cell<Option<Decision>>> = Rc::new(Cell::new(None));
    view().child(dynamic(move || {
        let (keep, reason, checked, poster, playing) = signature.get();
        let now = decision(keep, checked);
        let soak = shown.replace(Some(now)).is_some_and(|was| was != now);
        let mark = Mark {
            keep,
            reason,
            checked,
            soak,
        };
        if grid {
            media_card(group_index, &member, mark, poster.as_deref(), playing)
        } else {
            member_row(group_index, &member, mark, poster.as_deref(), playing)
        }
    }))
}

/// Groups where every member has a large preview render as a card grid.
fn is_grid_preview(thumb: &Thumb) -> bool {
    matches!(thumb, Thumb::Image | Thumb::Poster(_) | Thumb::PendingVideo)
}

/// What one copy's tile says about it.
fn decision(keep: bool, checked: bool) -> Decision {
    if keep {
        Decision::Keep
    } else if checked {
        Decision::Delete
    } else {
        Decision::Undecided
    }
}

/// Whether a preview stands for a clip, which a press can play.
fn is_video(thumb: &Thumb) -> bool {
    matches!(thumb, Thumb::Poster(_) | Thumb::PendingVideo)
}

/// A copy's preview, playable when it is a clip: a press on the preview plays
/// it in that same box, muted, and the Stop button puts the poster back — the
/// tile never changes size, and one video plays at a time (see
/// [`actions::toggle_play`]). The Play button in the tile's controls is the
/// keyboard's way to the same action.
fn playable_preview(thumb: &Thumb, path: &Path, size: PreviewSize, playing: bool) -> View {
    if playing && is_video(thumb) {
        let stop_path = path.to_path_buf();
        let player = preview_frame(PreviewSize::Card, PreviewFill::Picture)
            .child(video_player(path_source(path)));
        return if size == PreviewSize::Card {
            player.child(
                view()
                    .absolute()
                    .top(px(space::SM))
                    .right(px(space::SM))
                    .child(icon_button(icons::STOP, "Stop video", move || {
                        actions::toggle_play(&stop_path);
                    })),
            )
        } else {
            player
        };
    }
    let preview = member_preview(thumb, path, size);
    if is_video(thumb) {
        let play_path = path.to_path_buf();
        press_to_play(preview, move || actions::toggle_play(&play_path))
    } else {
        preview
    }
}

/// Where one copy stands: the keeper (and why), ticked to go, and whether
/// that has just changed (its wash soaks in).
#[derive(Clone, Copy)]
struct Mark {
    keep: bool,
    reason: Option<KeepReason>,
    checked: bool,
    soak: bool,
}

/// One copy in the grid layout: large preview, then the name, its folder and
/// the same controls a row carries. Only the edge and the badge move when the
/// keeper changes, so the group never reflows during a decision.
fn media_card(
    group_index: usize,
    member: &GroupMember,
    mark: Mark,
    poster: Option<&Path>,
    playing: bool,
) -> View {
    let thumb = thumb_for(member, poster);
    let video_member = is_video(&thumb);
    tile(decision(mark.keep, mark.checked))
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .w_full()
        .gap(px(space::SM))
        .child(playable_preview(
            &thumb,
            &member.path,
            PreviewSize::Card,
            playing,
        ))
        .child(
            view()
                .flex_col()
                .min_w(px(0.0))
                .gap(px(space::XXS))
                .child(name(member_name(member)).truncate())
                // Truncated, never allowed to widen the card: a deep folder
                // would otherwise stretch this card (and its preview) past
                // its neighbour.
                .child(caption(member_parent(member)).truncate()),
        )
        .child(member_actions(
            group_index,
            member,
            mark,
            // Videos only: a photo has nothing to play and gets no control.
            video_member.then_some(playing),
        ))
}

/// One copy in the list layout: thumbnail beside name, folder and controls.
/// Playing is the one deliberate expansion: a 72 pt thumbnail is not a usable
/// player, so the row grows to hold one while every other row keeps its box.
fn member_row(
    group_index: usize,
    member: &GroupMember,
    mark: Mark,
    poster: Option<&Path>,
    playing: bool,
) -> View {
    // Rendering only reads state: poster extraction is requested when a
    // group becomes visible (see `actions::request_group_posters`), never
    // while the tree is being built.
    let thumb = thumb_for(member, poster);
    let video_member = is_video(&thumb);
    let full = member_name(member);
    let shown = fit_text(&full, NAME_CHARS);
    let cut = shown != full;
    let cell = name(shown).truncate();
    let cell = if cut { cell.tooltip(hint(full)) } else { cell };
    let info = view()
        .flex_col()
        .flex_1()
        .min_w(px(0.0))
        .gap(px(space::XS))
        .child(cell)
        .child(caption(member_parent(member)))
        .child(member_actions(
            group_index,
            member,
            mark,
            video_member.then_some(playing),
        ));
    let surface = tile(decision(mark.keep, mark.checked)).gap(px(space::SM));
    if playing && video_member {
        return surface
            .flex_col()
            .child(playable_preview(
                &thumb,
                &member.path,
                PreviewSize::Thumb,
                true,
            ))
            .child(info);
    }
    surface
        .flex_row()
        .items_start()
        .child(playable_preview(
            &thumb,
            &member.path,
            PreviewSize::Thumb,
            false,
        ))
        .child(info)
}

/// The controls line both layouts use: the tick, size, similarity, the
/// protected badge, Play for a clip, and the keeper pick.
///
/// One verb pair says where every copy stands: the keeper wears "Keeping"
/// (a badge, not a checkbox — there is no choice to make about the copy
/// that stays) with why it stays beside it, a ticked copy reads "Delete",
/// an unticked one "Keep too". The keeper pick is the one action that
/// moves the mark: it makes this copy the keeper and puts the retiring
/// keeper on the delete list, where the amber edge and the Delete word show
/// it at once.
fn member_actions(
    group_index: usize,
    member: &GroupMember,
    mark: Mark,
    play: Option<bool>,
) -> View {
    let Mark {
        keep,
        reason,
        checked,
        soak,
    } = mark;
    let similarity = member
        .similarity
        .map(|s| format!("{:.0}% similar", s * 100.0));
    let toggle_path = member.path.clone();
    let keep_path = member.path.clone();
    let play_path = member.path.clone();
    let playing = play.unwrap_or(false);
    view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(space::SM))
        .child(if member.reference && !keep {
            // A locked copy is never deleted, so it has no tick to give: a
            // disabled checkbox would promise a choice that does not exist.
            badge("Protected", BadgeTone::Raised)
                .tooltip(hint("Inside a locked folder: this copy is never deleted."))
        } else if keep {
            // The copy the group keeps: a badge, not a checkbox, and why it
            // stays held close to it ("Keeping  the others look like
            // copies"), apart from the size and the rest of the line.
            view()
                .flex_row()
                .items_center()
                .gap(px(space::XS))
                .child(keeper_badge(soak))
                .child(caption(reason.map(|r| r.to_string()).unwrap_or_default()))
        } else {
            tick(
                if checked { "Delete" } else { "Keep too" },
                decision(keep, checked),
                soak,
                move || actions::toggle_selected(&toggle_path),
            )
        })
        .child(caption(util::human_bytes(member.size)).tabular_nums())
        .child(when(similarity.is_some(), move || {
            caption(similarity.clone().unwrap_or_default())
        }))
        // A protected duplicate already carries "Protected" in place of its
        // tick; a protected keeper carries it beside its own.
        .child(when(member.reference && keep, || {
            badge("Protected", BadgeTone::Raised)
        }))
        .child(when(play.is_some(), move || {
            let path = play_path.clone();
            let (markup, label) = if playing {
                (icons::STOP, "Stop video")
            } else {
                (icons::PLAY, "Play video")
            };
            icon_ghost_button(markup, label, move || {
                actions::toggle_play(&path);
            })
        }))
        .child(when(!keep, move || {
            let path = keep_path.clone();
            view()
                .child(quiet_button("Keep this one instead", move || {
                    actions::set_keeper(group_index, &path);
                }))
                .tooltip(hint(
                    "Keep this copy instead of the current keeper; the retiring keeper is marked to delete",
                ))
        }))
}

fn mode_workspace() -> View {
    view()
        .flex_col()
        .flex_1()
        .h_full()
        .gap(px(space::XL))
        .px(px(space::MA))
        .py(px(space::XXL))
        .child(
            view()
                .flex_col()
                .gap(px(space::LG))
                .child(mode_header())
                .child(dynamic(mode_actions)),
        )
        // Acting on the last findings lifts them off the paper, as a
        // duplicate clean-up does; a restore is not an ending.
        .child(ink_lift(
            || {
                app().with(|s| {
                    s.findings_done
                        .is_some_and(|done| done.went != FindingsWent::Restored)
                        && s.mode_result.as_ref().is_some_and(|r| r.is_empty())
                })
            },
            fast_scroll(&MODE_AT, dynamic(mode_list)),
        ))
}

/// What the cleanup workspace's toolbar renders from.
#[derive(Clone, Copy, PartialEq, Eq)]
struct ModeBar {
    shown: bool,
    act: actions::FindingsAct,
    plan: actions::FindingsPlan,
    trash: bool,
    can_undo: bool,
}

/// The cleanup workspace's toolbar: the one act its findings allow (delete
/// them, or strip the photos' metadata), Restore from trash after a trashed
/// delete, and Export report…. Duplicate folders are report-only, as on the
/// command line, so their bar holds the export alone.
fn mode_actions() -> View {
    let bar = computed(|| {
        app().with(|s| {
            let result = s.mode_result.as_ref();
            ModeBar {
                // With every finding gone the list's own card says what
                // happened and offers the restore; the bar has nothing left
                // to act on.
                shown: !s.scanning && result.is_some_and(|r| !r.is_empty()),
                act: result.map_or(actions::FindingsAct::ReportOnly, |r| {
                    actions::findings_act(r.mode)
                }),
                plan: actions::findings_plan(s),
                trash: s.trash,
                can_undo: !s.last_trashed.is_empty(),
            }
        })
    });
    let ModeBar {
        shown,
        act,
        plan,
        trash,
        can_undo,
    } = bar.get();
    if !shown {
        return view();
    }
    let size = util::human_bytes(plan.bytes);
    let count = plan.count;
    let mut decide = action_group();
    if count > 0 {
        decide = match act {
            actions::FindingsAct::Delete if trash => decide.child(
                danger_button(
                    format!("Delete {count} ({size})"),
                    actions::request_findings_act,
                )
                .tooltip(hint(format!(
                    "Move {} ({size}) to the trash",
                    counted(count as u64, "item", "items")
                ))),
            ),
            actions::FindingsAct::Delete => decide.child(
                danger_button(
                    format!("Delete {count} permanently ({size})"),
                    actions::request_findings_act,
                )
                .tooltip(hint(format!(
                    "Permanently delete {} ({size}). This can't be undone.",
                    counted(count as u64, "item", "items")
                ))),
            ),
            actions::FindingsAct::StripMetadata => decide.child(
                danger_button(
                    format!(
                        "Remove metadata from {} ({size})",
                        counted(count as u64, "photo", "photos")
                    ),
                    actions::request_findings_act,
                )
                .tooltip(hint("Rewrite the photos without their EXIF and XMP data")),
            ),
            actions::FindingsAct::ReportOnly => decide,
        };
    }
    if can_undo && actions::CAN_RESTORE {
        decide = decide.child(icon_secondary_button(
            icons::UNDO,
            "Restore from trash",
            actions::undo_delete,
        ));
    }
    view().w_full().child(action_bar(
        "Findings actions",
        vec![
            decide,
            action_group().child(
                icon_button(icons::EXPORT, "Export report…", actions::save_report)
                    .tooltip(hint("Export the findings as JSON, CSV or HTML")),
            ),
        ],
    ))
}

/// The cleanup scan's title row: what was looked for, how much was found, and
/// the space it holds as the answer figure. The line under the title says
/// what the findings allow: an act, or (duplicate folders) a review by hand.
fn mode_header() -> View {
    view()
        .flex_row()
        .flex_wrap()
        .items_end()
        .justify_between()
        .gap(px(space::LG))
        .child(title_block(
            heading(|| {
                app().with(|s| {
                    s.mode_result
                        .as_ref()
                        .map(|r| mode_title(r.mode))
                        .unwrap_or("Cleanup scan")
                        .to_string()
                })
            }),
            description(|| {
                app().with(|s| match s.mode_result.as_ref() {
                    Some(r) => {
                        let found = match actions::findings_act(r.mode) {
                            actions::FindingsAct::StripMetadata => {
                                counted(r.finding_count() as u64, "photo", "photos")
                            }
                            _ => counted(r.finding_count() as u64, "finding", "findings"),
                        };
                        let mut line = format!(
                            "{found} in {} scanned",
                            counted(r.files_scanned, "file", "files"),
                        );
                        let plan = actions::findings_plan(s);
                        if actions::findings_act(r.mode) == actions::FindingsAct::ReportOnly {
                            line.push_str(" · listed only: check each one and remove it yourself");
                        } else if plan.protected > 0 {
                            line.push_str(&format!(
                                " · {} in a protected folder stay",
                                counted(plan.protected as u64, "item", "items")
                            ));
                        }
                        line
                    }
                    None => "Run a cleanup scan to see findings.".to_string(),
                })
            }),
        ))
        .child(
            view()
                .flex_row()
                .items_end()
                .gap(px(space::XL))
                .child(scan_again_button())
                .child(answer_figure("Reclaimable", || {
                    app().with(|s| {
                        s.mode_result
                            .as_ref()
                            .map_or_else(String::new, |r| util::human_bytes(r.reclaimable()))
                    })
                })),
        )
}

/// Items drawn in the findings' confirmation before it counts the rest.
const FINDINGS_ROWS: usize = 8;

/// The findings' confirmation: the first items the act touches, a count of
/// the rest, and what a protected folder keeps. Nothing changes until its
/// commit is pressed, and every file is checked against the scan first.
pub(crate) fn findings_confirm() -> View {
    let (act, plan, trash, rows) = app().with_untracked(|s| {
        (
            s.mode_result
                .as_ref()
                .map_or(actions::FindingsAct::ReportOnly, |r| {
                    actions::findings_act(r.mode)
                }),
            actions::findings_plan(s),
            s.trash,
            actions::findings_rows(s, FINDINGS_ROWS),
        )
    });
    let count = plan.count;
    let size = util::human_bytes(plan.bytes);
    let mut body = view().flex_col().w_full().gap(px(space::MD));
    for (path, bytes) in &rows {
        let file_name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let folder = path
            .parent()
            .map_or_else(String::new, |p| p.display().to_string());
        body = body.child(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::SM))
                .child(
                    view()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .gap(px(space::XXS))
                        .child(panel_title(file_name).truncate())
                        .child(caption(fit_text(&folder, PLAN_PATH_CHARS)).truncate()),
                )
                .child(caption(util::human_bytes(*bytes))),
        );
    }
    if count > rows.len() {
        body = body.child(caption(format!(
            "… and {} more",
            counted((count - rows.len()) as u64, "item", "items")
        )));
    }
    let items = counted(count as u64, "item", "items");
    let checked =
        "Each is checked against the scan one last time first; anything that changed stays.";
    let kept = if plan.protected > 0 {
        format!(
            " {} in a protected folder {}.",
            counted(plan.protected as u64, "item", "items"),
            if plan.protected == 1 { "stays" } else { "stay" }
        )
    } else {
        String::new()
    };
    let (heading, summary, label) = match act {
        actions::FindingsAct::StripMetadata => {
            let photos = counted(count as u64, "photo", "photos");
            (
                format!("Remove metadata from {photos}?"),
                format!(
                    "Their EXIF and XMP data (camera, place, dates) is removed and each photo is rewritten in place; {size} comes back. The pictures themselves do not change. This can't be undone. {checked}{kept}"
                ),
                "Remove metadata".to_string(),
            )
        }
        _ if trash => (
            format!("Move {items} to the trash?"),
            format!(
                "{items} ({size}) will move to the trash. You can put them back from the trash until it is emptied. {checked}{kept}"
            ),
            format!("Move {count} to the trash"),
        ),
        _ => (
            format!("Delete {items} permanently?"),
            format!(
                "{items} ({size}) will be deleted permanently. This can't be undone. {checked}{kept}"
            ),
            format!("Delete {count} permanently"),
        ),
    };
    let mut buttons = view().child(secondary_button("Cancel", actions::cancel_findings_act));
    if count > 0 && act != actions::FindingsAct::ReportOnly {
        buttons = buttons.child(danger_button(label, actions::confirm_findings_act));
    }
    alert_dialog(
        heading,
        summary,
        fast_scroll_max(&FINDINGS_AT, px(DELETE_PLAN_MAX_H), body),
        buttons,
        actions::cancel_findings_act,
    )
}

fn mode_list() -> View {
    let Some(result) = app().with(|s| s.mode_result.clone()) else {
        return view().child(caption("Run a cleanup scan to see findings."));
    };
    let mut list = view().flex_col().gap(px(space::MD));
    if app().with(|s| s.scanning) {
        list = list.child(scan_card());
    }
    if result.is_empty() {
        if let Some(done) = app().with(|s| s.findings_done) {
            return list.child(findings_cleared_card(done));
        }
        list = list.child(nothing_found_card(
            "Nothing to clean up",
            "The scanned folders came back clean. Add a folder, or scan again after changing the threshold.",
            false,
        ));
        return list;
    }
    // A cleanup scan over a large tree returns findings by the thousand
    // ("empty files" in a download folder, "temp files" in a home
    // directory). Rows are built for what is rendered, so the list takes a
    // page at a time, the same rule the duplicate groups already follow.
    let limit = app().with(|s| s.mode_limit);
    if !result.findings.is_empty() {
        // Single findings are one list: one card, rows split by rules, the
        // sizes on one right edge — not a stack of identical cards.
        let mut rows = card().p(px(space::XS));
        for (index, finding) in result.findings.iter().take(limit).enumerate() {
            if index > 0 {
                rows = rows.child(view().px(px(space::SM)).child(divider()));
            }
            rows = rows.child(mode_finding_row(finding));
        }
        list = list.child(rows);
    }
    for group in result.groups.iter().take(limit) {
        list = list.child(mode_group_card(group));
    }
    for error in result.errors.iter().take(limit) {
        list = list.child(error_notice(error.clone()));
    }
    let remaining = result.findings.len().saturating_sub(limit)
        + result.groups.len().saturating_sub(limit)
        + result.errors.len().saturating_sub(limit);
    if remaining > 0 {
        list = list.child(view().flex_row().justify_center().child(ghost_button(
            format!("Show more ({remaining} remaining)"),
            actions::show_more_findings,
        )));
    }
    list
}

/// A path that may not fit: cut to the room it has, with the whole path in
/// a tooltip when it was cut. In ink it is a row's identity at body size;
/// muted it is a member listed under a group at caption size.
/// About how many characters of a path a full-width row shows before the
/// front is cut.
const PATH_CHARS: usize = 110;

/// About how many characters of a file name a list row shows before the
/// name is cut and moved into a tooltip.
const NAME_CHARS: usize = 120;

fn path_cell(path: &Path, width: usize, ink: Ink) -> Text {
    let full = path.to_string_lossy().to_string();
    let shown = fit_text(&full, width);
    let truncated = shown != full;
    let cell = match ink {
        Ink::Text => body(shown).truncate(),
        Ink::Muted => caption(shown).truncate(),
    };
    if truncated {
        cell.tooltip(hint(full))
    } else {
        cell
    }
}

/// One finding: its kind, its path over its detail, its size on the right
/// edge every row shares.
fn mode_finding_row(finding: &crate::modes::Finding) -> View {
    view()
        .flex_row()
        .items_center()
        .gap(px(space::MD))
        .px(px(space::SM))
        .py(px(space::SM))
        .child(badge(
            finding.kind,
            BadgeTone::Kind(finding_kind(finding.kind)),
        ))
        .child(
            view()
                .flex_col()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(space::XXS))
                .child(path_cell(&finding.path, PATH_CHARS, Ink::Text))
                .child(caption(finding.detail.clone()).truncate()),
        )
        .child(amount(util::human_bytes(finding.size)))
}

/// A group of findings (a duplicate folder, a run of duplicate tracks): its
/// label and the space it holds, then its members as plain paths.
fn mode_group_card(group: &crate::modes::FindingGroup) -> View {
    card()
        .gap(px(space::SM))
        .p(px(space::LG))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(space::MD))
                .child(name(group.label.clone()).flex_1().truncate())
                .child(amount(format!(
                    "{} reclaimable",
                    util::human_bytes(group.reclaimable)
                ))),
        )
        .child(caption(group.detail.clone()))
        .children(
            group
                .members
                .iter()
                .map(|member| path_cell(member, PATH_CHARS, Ink::Muted)),
        )
}
