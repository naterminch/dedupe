//! The compare view: two copies of one group side by side, for a close look
//! before choosing which one stays.
//!
//! Similar media (the same photo re-saved, cropped or re-encoded) can differ
//! in ways a thumbnail hides. The view opens over the whole window with the
//! group's keeper on the left and one other copy on the right; a group of
//! more than two pages its other copies through the right pane. Photos zoom
//! and pan together: one lens (how far in, and which point of the picture
//! sits at the centre, as fractions of its size) drives every pane, so the
//! same region of two copies of different sizes is in view at once. Clips
//! play, pause and seek together.
//!
//! Every decision taken here goes through the results list's own actions
//! ([`actions::set_keeper`], [`actions::toggle_selected`]): the view is a
//! review aid and owns no state of its own beyond the lens.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use nsh_core::keymap::When;
use nsh_core::prelude::*;

use crate::matching::{Group, GroupMember};
use crate::media::{self, MediaKind};
use crate::ui::actions;
use crate::ui::palette::space;
use crate::ui::results::{
    Mark, THUMB_EXTS, answer_figure, decision, decision_mark, file_kind, group_header,
    keep_instead_button, kind_label, member_name,
};
use crate::ui::state::{Comparing, app};
use crate::ui::widgets::{
    BadgeTone, Decision, action_bar, action_group, amount, badge, caption, card, description,
    divider, fading_in, field_label, heading, hint, icon_button, icon_secondary_button, icons,
    live_tile, name, scrubber, secondary_button, segmented_names, synced_player, title_block,
    viewer_frame, viewer_image, window_sheet,
};
use crate::ui_helpers::fit_text;
use crate::util;

// --- The lens ---------------------------------------------------------------

/// How far one zoom step (a key, a button, a wheel notch) goes.
pub(crate) const ZOOM_STEP: f32 = 1.25;

/// The closest look: eight screen pixels to one picture pixel.
pub(crate) const MAX_SCALE: f32 = 8.0;

/// Points of wheel travel in one mouse notch (three 16 pt lines).
const WHEEL_NOTCH: f32 = 48.0;

/// The zoom every pane shares: how far in from "fit" (1 shows the whole
/// picture), and the point of the picture at the pane's centre, as fractions
/// of its width and height. Fractions, not pixels, so two copies of
/// different sizes show the same region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Lens {
    pub(crate) zoom: f32,
    pub(crate) cx: f32,
    pub(crate) cy: f32,
}

impl Default for Lens {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            cx: 0.5,
            cy: 0.5,
        }
    }
}

/// One pane's geometry, in points: its viewer's size and its picture's
/// laid-out size (one picture pixel per device pixel).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct Frame {
    pub(crate) view: (f32, f32),
    pub(crate) picture: (f32, f32),
}

impl Frame {
    fn ready(self) -> bool {
        self.view.0 > 0.0 && self.view.1 > 0.0 && self.picture.0 > 0.0 && self.picture.1 > 0.0
    }

    /// The scale that shows the whole picture in the viewer.
    pub(crate) fn fit(self) -> f32 {
        if !self.ready() {
            return 1.0;
        }
        (self.view.0 / self.picture.0).min(self.view.1 / self.picture.1)
    }

    /// The picture's scale under `lens`: 1 shows it pixel for pixel.
    pub(crate) fn scale(self, lens: Lens) -> f32 {
        self.fit() * lens.zoom
    }

    /// Where the picture's top-left corner lands under `lens`. A picture
    /// smaller than the viewer sits in its middle; a larger one never pulls
    /// an empty margin into view.
    pub(crate) fn offset(self, lens: Lens) -> (f32, f32) {
        let scale = self.scale(lens);
        let (w, h) = (self.picture.0 * scale, self.picture.1 * scale);
        let cx = hold(lens.cx, self.view.0, w);
        let cy = hold(lens.cy, self.view.1, h);
        (self.view.0 / 2.0 - cx * w, self.view.1 / 2.0 - cy * h)
    }

    /// The zoom range in this pane: from the whole picture (or its own
    /// pixels, for a picture smaller than the pane) to [`MAX_SCALE`].
    fn zoom_range(self) -> (f32, f32) {
        let fit = self.fit();
        let least = (1.0 / fit).min(1.0);
        (least, (MAX_SCALE / fit).max(1.0))
    }
}

/// A centre fraction held so the picture covers the viewer along that axis;
/// a picture narrower than the viewer is centred.
fn hold(centre: f32, view: f32, extent: f32) -> f32 {
    if extent <= view {
        0.5
    } else {
        let half = view / (2.0 * extent);
        centre.clamp(half, 1.0 - half)
    }
}

impl Lens {
    /// Zoomed by `factor` about `at`, a point in `frame`'s viewer: the part of
    /// the picture under it stays under it.
    pub(crate) fn zoomed(self, frame: Frame, factor: f32, at: (f32, f32)) -> Self {
        if !frame.ready() {
            return self;
        }
        let (least, most) = frame.zoom_range();
        let zoom = (self.zoom * factor).clamp(least, most);
        let before = frame.scale(self);
        let after = frame.fit() * zoom;
        let (pw, ph) = frame.picture;
        let (vw, vh) = frame.view;
        let cx = hold(self.cx, vw, pw * before);
        let cy = hold(self.cy, vh, ph * before);
        let (dx, dy) = (at.0 - vw / 2.0, at.1 - vh / 2.0);
        let (u, v) = (cx + dx / (pw * before), cy + dy / (ph * before));
        Self {
            zoom,
            cx: hold(u - dx / (pw * after), vw, pw * after),
            cy: hold(v - dy / (ph * after), vh, ph * after),
        }
    }

    /// Zoomed by `factor` about the viewer's centre: a key or a button.
    pub(crate) fn stepped(self, frame: Frame, factor: f32) -> Self {
        self.zoomed(frame, factor, (frame.view.0 / 2.0, frame.view.1 / 2.0))
    }

    /// Moved by a drag of `(dx, dy)` points in `frame`'s viewer: the picture
    /// follows the pointer.
    pub(crate) fn panned(self, frame: Frame, (dx, dy): (f32, f32)) -> Self {
        if !frame.ready() {
            return self;
        }
        let scale = frame.scale(self);
        let (w, h) = (frame.picture.0 * scale, frame.picture.1 * scale);
        let cx = hold(self.cx, frame.view.0, w) - dx / w;
        let cy = hold(self.cy, frame.view.1, h) - dy / h;
        Self {
            zoom: self.zoom,
            cx: hold(cx, frame.view.0, w),
            cy: hold(cy, frame.view.1, h),
        }
    }

    /// `frame`'s picture pixel for pixel ("100%"), about the same centre.
    pub(crate) fn actual(self, frame: Frame) -> Self {
        if !frame.ready() {
            return self;
        }
        self.stepped(frame, 1.0 / (frame.fit() * self.zoom))
    }
}

// --- What each copy is ------------------------------------------------------

/// What the details table reads off a copy beyond its group entry: its size
/// in pixels, its length, its format, and its camera and date taken.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Facts {
    pub(crate) pixels: Option<(u32, u32)>,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) format: String,
    pub(crate) camera: Option<String>,
    pub(crate) taken: Option<String>,
}

/// Read a copy's facts: a photo's header and EXIF block, a clip's probe
/// (the scan's own when it ran one). Blocking: file reads and, for a clip
/// the scan did not probe, ffprobe.
fn read_facts(member: &GroupMember) -> Facts {
    let ext = member
        .path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_uppercase)
        .unwrap_or_default();
    match media::classify(&member.path) {
        MediaKind::Image => {
            let photo = crate::modes::photo_info(&member.path);
            let pixels = photo.as_ref().map(|p| (p.width, p.height)).or_else(|| {
                member
                    .media
                    .as_ref()
                    .and_then(|m| Some((m.width?, m.height?)))
            });
            let (camera, taken) = photo.map(|p| (p.camera, p.taken)).unwrap_or_default();
            Facts {
                pixels,
                duration_ms: None,
                format: ext,
                camera,
                taken,
            }
        }
        MediaKind::Video => {
            let info = member
                .media
                .clone()
                .or_else(|| media::probe_media(&member.path));
            let codec = info
                .as_ref()
                .and_then(|i| i.codec.as_deref())
                .map(str::to_ascii_uppercase);
            Facts {
                pixels: info.as_ref().and_then(|i| Some((i.width?, i.height?))),
                duration_ms: info.as_ref().and_then(|i| i.duration_ms),
                format: match codec {
                    Some(codec) => format!("{ext} · {codec}"),
                    None => ext,
                },
                camera: None,
                taken: None,
            }
        }
        MediaKind::Other => Facts {
            format: ext,
            ..Facts::default()
        },
    }
}

/// One line of the details table: what it is, each pane's value, and which
/// pane (if either) has the better one, in words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FactRow {
    pub(crate) label: &'static str,
    pub(crate) values: [String; 2],
    pub(crate) better: Option<(usize, &'static str)>,
}

impl FactRow {
    /// The two panes disagree here.
    pub(crate) fn differs(&self) -> bool {
        self.values[0] != self.values[1]
    }
}

/// The dash for a value a copy does not have.
const NONE: &str = "—";

/// The details table of a pair. Lines neither copy has a value for are left
/// out (a photo has no length, a PNG rarely a camera). The better value is
/// named only where "better" means something to the picture: more detail,
/// a longer clip. A smaller file or a newer date is a fact, not a verdict.
pub(crate) fn fact_rows(
    members: [&GroupMember; 2],
    facts: [&Facts; 2],
    exact: bool,
) -> Vec<FactRow> {
    let mut rows = Vec::new();
    let mut push = |label, values: [String; 2], better| {
        if values.iter().any(|v| v != NONE) {
            rows.push(FactRow {
                label,
                values,
                better,
            });
        }
    };
    let both = |f: &dyn Fn(usize) -> String| [f(0), f(1)];
    let pixels = facts.map(|f| f.pixels.map(|(w, h)| u64::from(w) * u64::from(h)));
    push(
        "Picture size",
        both(&|i| {
            facts[i]
                .pixels
                .map_or_else(|| NONE.to_string(), |(w, h)| format!("{w} × {h}"))
        }),
        larger(pixels).map(|i| (i, "More detail")),
    );
    let lengths = facts.map(|f| f.duration_ms);
    push(
        "Length",
        both(&|i| {
            facts[i]
                .duration_ms
                .map_or_else(|| NONE.to_string(), util::format_duration_ms)
        }),
        // A second's difference is a re-encode's rounding, not a longer clip.
        larger(lengths.map(|l| l.map(|ms| ms / 1000))).map(|i| (i, "Longer")),
    );
    push(
        "File size",
        both(&|i| util::human_bytes(members[i].size)),
        None,
    );
    push(
        "Format",
        both(&|i| {
            if facts[i].format.is_empty() {
                NONE.to_string()
            } else {
                facts[i].format.clone()
            }
        }),
        None,
    );
    push(
        "Taken",
        both(&|i| {
            facts[i]
                .taken
                .as_deref()
                .map_or_else(|| NONE.to_string(), exif_moment)
        }),
        None,
    );
    push(
        "Camera",
        both(&|i| facts[i].camera.clone().unwrap_or_else(|| NONE.to_string())),
        None,
    );
    push(
        "Modified",
        both(&|i| {
            members[i]
                .mtime_secs
                .map_or_else(|| NONE.to_string(), calendar_day)
        }),
        None,
    );
    push(
        "Similarity",
        both(&|i| {
            if exact {
                "Identical".to_string()
            } else {
                members[i]
                    .similarity
                    .map_or_else(|| NONE.to_string(), |s| format!("{:.0}%", s * 100.0))
            }
        }),
        None,
    );
    push(
        "Folder",
        both(&|i| {
            members[i]
                .path
                .parent()
                .map_or_else(|| NONE.to_string(), |p| p.display().to_string())
        }),
        None,
    );
    rows
}

/// Which of two amounts is larger, when both are known and they differ.
fn larger(values: [Option<u64>; 2]) -> Option<usize> {
    match values {
        [Some(a), Some(b)] if a > b => Some(0),
        [Some(a), Some(b)] if b > a => Some(1),
        _ => None,
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A file time as the day it falls on ("26 Nov 2026"). The day, in UTC:
/// the hour would need the local zone, which the day rarely does.
pub(crate) fn calendar_day(unix_secs: i64) -> String {
    let (y, m, d) = civil_from_days(unix_secs.div_euclid(86_400));
    format!("{d} {} {y}", MONTHS[(m - 1) as usize])
}

/// An EXIF date ("2024:05:01 12:34:56", the camera's own clock) as
/// "1 May 2024, 12:34"; anything else as written.
pub(crate) fn exif_moment(raw: &str) -> String {
    let parse = || -> Option<String> {
        let (date, time) = raw.trim().split_once(' ')?;
        let mut date = date.split(':').map(str::parse::<u32>);
        let (y, m, d) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
        let month = MONTHS.get(usize::try_from(m.checked_sub(1)?).ok()?)?;
        let hm = time.get(..5)?;
        (d > 0).then(|| format!("{d} {month} {y}, {hm}"))
    };
    parse().unwrap_or_else(|| raw.trim().to_string())
}

/// Days since 1970-01-01 to a proleptic Gregorian `(year, month, day)`
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

// --- The view ---------------------------------------------------------------

/// The keys of the open view: Left and Right step through the groups, `+`,
/// `-`, `0` (fit) and `1` (100%) zoom. Escape is the root's (it closes the
/// topmost layer) and the dialog's own.
const KEYS: KeyOptions = KeyOptions {
    when: When::NotEditing,
    repeat: true,
    pass_through: false,
    for_focus: false,
};

/// The lens and the panes' geometry, shared by the keys, the toolbar and the
/// panes. `glide` says whether the last change came from a key or a button
/// (it eases in) or from the pointer (it follows at once).
#[derive(Clone, Copy)]
struct Zoom {
    lens: Signal<Lens>,
    frames: [Signal<Frame>; 2],
    glide: Signal<bool>,
}

impl Zoom {
    /// The pane zoom limits and 100% are read against: the keeper's, the
    /// copy the comparison is anchored on.
    fn lead(self) -> Frame {
        self.frames[0].get_untracked()
    }

    fn step(self, factor: f32) {
        let frame = self.lead();
        self.glide.set(true);
        self.lens.update(|l| *l = l.stepped(frame, factor));
    }

    fn fit(self) {
        self.glide.set(true);
        self.lens.set(Lens::default());
    }

    fn actual(self) {
        let frame = self.lead();
        self.glide.set(true);
        self.lens.update(|l| *l = l.actual(frame));
    }
}

/// The compare layer over the whole window, while a pair is open. Rebuilt
/// only when the view opens: stepping to another group or copy swaps the
/// page inside it, so focus and the window layer stay put.
pub(crate) fn compare_layer() -> View {
    let zoom = Zoom {
        lens: signal(Lens::default()),
        frames: [signal(Frame::default()), signal(Frame::default())],
        glide: signal(false),
    };
    on_key_with("left", KEYS, |_| actions::compare_step_group(-1));
    on_key_with("right", KEYS, |_| actions::compare_step_group(1));
    on_key_with("plus", KEYS, move |_| zoom.step(ZOOM_STEP));
    on_key_with("shift+plus", KEYS, move |_| zoom.step(ZOOM_STEP));
    on_key_with("minus", KEYS, move |_| zoom.step(1.0 / ZOOM_STEP));
    on_key_with("0", KEYS, move |_| zoom.fit());
    on_key_with("1", KEYS, move |_| zoom.actual());
    window_sheet(
        "Compare copies",
        actions::close_compare,
        dynamic(move || {
            let pair = computed(|| app().with(|s| s.comparing.clone()));
            match pair.get() {
                Some(pair) => page(&pair, zoom),
                None => view(),
            }
        }),
    )
}

/// One pair's page: the title row with the group's figure, the toolbar, and
/// the two panes over their details.
fn page(pair: &Comparing, zoom: Zoom) -> View {
    let Some(group) =
        app().with_untracked(|s| s.groups.iter().find(|g| g.index == pair.group).cloned())
    else {
        return view();
    };
    let member = |path: &Path| group.members.iter().find(|m| m.path == path).cloned();
    let (Some(left), Some(right)) = (member(&pair.left), member(&pair.right)) else {
        return view();
    };
    // A new pair starts whole: its own pictures at fit.
    untracked(|| {
        zoom.lens.set(Lens::default());
        zoom.glide.set(false);
        for frame in zoom.frames {
            frame.update(|f| f.picture = (0.0, 0.0));
        }
    });
    let facts = {
        let (left, right) = (left.clone(), right.clone());
        resource(
            || Some(()),
            move |(), _| {
                let (left, right) = (left.clone(), right.clone());
                async move {
                    blocking(move || [read_facts(&left), read_facts(&right)])
                        .await
                        .map_err(|_| ())
                }
            },
        )
    };
    let videos = [&left, &right].map(|m| media::classify(&m.path) == MediaKind::Video);
    let transport = Transport::new();
    let arrive = fading_in();
    view()
        .flex_col()
        .flex_1()
        .h_full()
        .min_h(px(0.0))
        .gap(px(space::LG))
        .px(px(space::MA))
        .py(px(space::XL))
        .child(title_row(&group))
        .child(toolbar(
            &group,
            &pair.left,
            &pair.right,
            zoom,
            videos,
            transport,
        ))
        .child(
            card()
                .flex_1()
                .min_h(px(0.0))
                .p(px(space::MD))
                .gap(px(space::MD))
                .child(
                    view()
                        .flex_row()
                        .flex_1()
                        .min_h(px(0.0))
                        .gap(px(space::MD))
                        .child(pane(0, group.index, left.clone(), zoom, facts, transport))
                        .child(pane(1, group.index, right.clone(), zoom, facts, transport)),
                )
                .child(divider())
                .child(details(left, right, group.similarity.is_none(), facts)),
        )
        .bind_motion(&arrive)
}

/// "Compare copies", what the group is, and the space its ticked copies give
/// back: the group's share of Reclaimable, moving as the decisions do.
fn title_row(group: &Group) -> View {
    let index = group.index;
    let kind = group
        .members
        .first()
        .map_or("File", |m| kind_label(file_kind(&m.path)));
    view()
        .flex_row()
        .flex_wrap()
        .justify_between()
        .items_end()
        .gap(px(space::LG))
        .child(title_block(
            heading("Compare copies"),
            description(format!("{kind} · {}", group_header(group))),
        ))
        .child(
            answer_figure("Reclaimable in this group", move || {
                app().with(|s| {
                    let bytes = s.groups.iter().find(|g| g.index == index).map_or(0, |g| {
                        g.members
                            .iter()
                            .filter(|m| m.deletable() && s.selected.contains(&m.path))
                            .map(|m| m.size)
                            .sum()
                    });
                    util::human_bytes(bytes)
                })
            })
            .tooltip(hint(
                "The space this group's ticked copies would give back if you deleted them now",
            )),
        )
}

/// The view's commands: the groups either side, the zoom or the shared
/// transport, the copy beside the keeper in a larger group, and Close.
fn toolbar(
    group: &Group,
    left: &Path,
    right: &Path,
    zoom: Zoom,
    videos: [bool; 2],
    transport: Transport,
) -> impl IntoElement {
    let travel = action_group()
        .child(
            icon_secondary_button(icons::ARROW_LEFT, "Previous group", || {
                actions::compare_step_group(-1);
            })
            .tooltip(hint("The previous group (Left arrow)")),
        )
        .child(
            icon_secondary_button(icons::ARROW_RIGHT, "Next group", || {
                actions::compare_step_group(1);
            })
            .tooltip(hint("The next group (Right arrow)")),
        )
        .child(caption(|| {
            app().with(|s| {
                actions::compare_position(s)
                    .map(|(at, of)| format!("Group {at} of {of}"))
                    .unwrap_or_default()
            })
        }));
    let look = if videos.iter().any(|v| *v) {
        transport.controls()
    } else {
        zoom_controls(zoom)
    };
    let mut close = action_group();
    if group.members.len() > 2 {
        // The copies that can sit beside the keeper, by name: a larger group
        // pages through them against the same left pane.
        let others: Vec<_> = group.members.iter().filter(|m| m.path != left).collect();
        let selected = others.iter().position(|m| m.path == right).unwrap_or(0);
        let names = others
            .iter()
            .map(|m| SharedString::from(fit_text(&member_name(m), COPY_NAME_CHARS)))
            .collect();
        let paths: Vec<_> = others.iter().map(|m| m.path.clone()).collect();
        close = close
            .child(field_label("Beside the keeper"))
            .child(segmented_names(names, selected, move |i| {
                if let Some(path) = paths.get(i) {
                    actions::compare_beside(path);
                }
            }));
    }
    let close = close.child(
        secondary_button("Close", actions::close_compare).tooltip(hint("Back to the list (Esc)")),
    );
    action_bar("Compare actions", vec![travel, look, close])
}

/// The longest copy name a segment shows; the end of a name is what tells
/// two copies apart, so the cut keeps it.
const COPY_NAME_CHARS: usize = 24;

/// Zoom out, Fit, 100%, Zoom in.
fn zoom_controls(zoom: Zoom) -> View {
    action_group()
        .child(icon_button(icons::MINUS, "Zoom out", move || {
            zoom.step(1.0 / ZOOM_STEP);
        }))
        .child(
            secondary_button("Fit", move || zoom.fit())
                .tooltip(hint("Show each picture whole (0)")),
        )
        .child(
            secondary_button("100%", move || zoom.actual())
                .tooltip(hint("Show the keeper pixel for pixel (1)")),
        )
        .child(icon_button(icons::PLUS, "Zoom in", move || {
            zoom.step(ZOOM_STEP);
        }))
}

/// One shared transport for the pair's clips: both play and pause together,
/// and the scrubber seeks both.
#[derive(Clone, Copy)]
struct Transport {
    paused: Signal<bool>,
    at: Signal<f64>,
    length: Signal<f64>,
    players: [ElementRef; 2],
}

impl Transport {
    fn new() -> Self {
        Self {
            paused: signal(true),
            at: signal(0.0),
            length: signal(0.0),
            players: [element_ref(), element_ref()],
        }
    }

    fn seek(self, seconds: f64) {
        self.at.set(seconds);
        for player in self.players {
            player.seek(seconds);
        }
    }

    fn controls(self) -> View {
        action_group()
            .child(dynamic(move || {
                let paused = self.paused.get();
                let (markup, label) = if paused {
                    (icons::PLAY, "Play both")
                } else {
                    (icons::PAUSE, "Pause both")
                };
                icon_button(markup, label, move || self.paused.update(|p| *p = !*p))
            }))
            .child(scrubber(
                "Position in both clips",
                move || self.at.get(),
                move || self.length.get(),
                move |seconds| self.seek(seconds),
            ))
            .child(
                caption(move || {
                    let ms = |s: f64| (s.max(0.0) * 1000.0) as u64;
                    format!(
                        "{} / {}",
                        util::format_duration_ms(ms(self.at.get())),
                        util::format_duration_ms(ms(self.length.get()))
                    )
                })
                .tabular_nums(),
            )
    }
}

/// One copy: its name and decision over its viewer. The tile's edge and the
/// decision follow the keeper and the tick as they change; the picture or
/// the clip stays mounted through them.
fn pane(
    side: usize,
    group_index: usize,
    member: GroupMember,
    zoom: Zoom,
    facts: Resource<[Facts; 2], ()>,
    transport: Transport,
) -> View {
    let path = member.path.clone();
    let standing = computed(move || {
        app().with(|s| {
            let group = s.groups.iter().find(|g| g.index == group_index);
            let current = group.and_then(|g| g.members.iter().find(|m| m.path == path));
            (
                current.is_some_and(|m| m.keep),
                group.zip(current).and_then(|(g, m)| g.reason_for(m)),
                s.selected.contains(&path),
            )
        })
    });
    let shown: Rc<Cell<Option<Decision>>> = Rc::new(Cell::new(None));
    let head_member = member.clone();
    let head = view()
        .flex_row()
        .flex_wrap()
        .items_center()
        .gap(px(space::SM))
        .child(
            name(member_name(&member))
                .flex_1()
                .min_w(px(0.0))
                .truncate(),
        )
        .child(dynamic(move || {
            let (keep, reason, checked) = standing.get();
            let now = decision(keep, checked);
            let soak = shown.replace(Some(now)).is_some_and(|was| was != now);
            let line = view()
                .flex_row()
                .items_center()
                .gap(px(space::SM))
                .child(decision_mark(
                    &head_member,
                    Mark {
                        keep,
                        reason,
                        checked,
                        soak,
                    },
                ));
            if keep {
                line
            } else {
                line.child(keep_instead_button(group_index, &head_member.path))
            }
        }));
    let viewer = match media::classify(&member.path) {
        MediaKind::Video => clip_viewer(side, &member, transport),
        _ if shows_picture(&member.path) => picture_viewer(side, &member, zoom, facts),
        _ => viewer_frame()
            .items_center()
            .justify_center()
            .child(caption("No preview for this format")),
    };
    live_tile(move || {
        let (keep, _, checked) = standing.get();
        decision(keep, checked)
    })
    .flex_col()
    .flex_1()
    .min_w(px(0.0))
    .gap(px(space::SM))
    .child(head)
    .child(viewer)
}

/// Whether the image element can draw this photo (HEIC, TIFF and the like
/// it cannot).
fn shows_picture(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| THUMB_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        && path.is_file()
}

/// A photo under the shared lens. The wheel zooms about the pointer, a drag
/// pans; both move every pane, since they move the lens.
fn picture_viewer(
    side: usize,
    member: &GroupMember,
    zoom: Zoom,
    facts: Resource<[Facts; 2], ()>,
) -> View {
    let frame = zoom.frames[side];
    let lens = zoom.lens;
    let source = member.path.to_string_lossy().into_owned();
    let dragged_from: Rc<Cell<Option<(f32, f32)>>> = Rc::new(Cell::new(None));
    let released = Rc::clone(&dragged_from);
    viewer_frame()
        .cursor(Cursor::Grab)
        .role(AccessibilityRole::Image)
        .label(format!("{}, zoomable", member_name(member)))
        .on_layout(move |bounds: &Bounds<Points>| {
            frame.update(|f| f.view = (bounds.size.width.0, bounds.size.height.0));
        })
        .on_wheel(move |e: &WheelEvent| {
            let travel = e.delta.points(px(WHEEL_NOTCH / 3.0)).y.0;
            if travel == 0.0 {
                return;
            }
            let factor = ZOOM_STEP.powf(-travel / WHEEL_NOTCH);
            let at = (e.position.x.0, e.position.y.0);
            let here = frame.get_untracked();
            zoom.glide.set(false);
            lens.update(|l| *l = l.zoomed(here, factor, at));
        })
        .on_mouse_move(move |e: &MouseEvent| {
            let at = (e.position.x.0, e.position.y.0);
            if e.pressed_button.is_none() {
                dragged_from.set(None);
                return;
            }
            if let Some(from) = dragged_from.replace(Some(at)) {
                let here = frame.get_untracked();
                zoom.glide.set(false);
                lens.update(|l| *l = l.panned(here, (at.0 - from.0, at.1 - from.1)));
            }
        })
        .on_mouse_up(move |_: &MouseEvent| released.set(None))
        .child(dynamic(move || {
            // The picture's own size lays it out; until the header is read
            // the frame stays empty rather than guessing.
            let pixels = facts.with(|f| f.and_then(|f| f[side].pixels));
            let Some((w, h)) = pixels else {
                return view();
            };
            let ratio = untracked(pixel_ratio).max(1.0);
            let size = (w as f32 / ratio, h as f32 / ratio);
            untracked(|| frame.update(|f| f.picture = size));
            viewer_image(
                source.clone(),
                size,
                move || frame.get().scale(lens.get()),
                (
                    move || frame.get().offset(lens.get()).0,
                    move || frame.get().offset(lens.get()).1,
                ),
                move || zoom.glide.get(),
            )
        }))
        .child(
            view()
                .absolute()
                .left(px(space::SM))
                .bottom(px(space::SM))
                .pointer_events(PointerEvents::None)
                .child(badge(
                    move || {
                        let here = frame.get();
                        if here.ready() {
                            format!("{:.0}%", here.scale(lens.get()) * 100.0)
                        } else {
                            String::new()
                        }
                    },
                    BadgeTone::Raised,
                ))
                .tooltip(hint(
                    "How large this copy is shown: 100% is pixel for pixel",
                )),
        )
}

/// A clip that plays with its neighbour: paused at its first frame until
/// Play both, and seeked with it. The left clip reports the position.
fn clip_viewer(side: usize, member: &GroupMember, transport: Transport) -> View {
    let source = member.path.to_string_lossy().into_owned();
    let player = synced_player(source)
        .element_ref(&transport.players[side])
        .paused(move || transport.paused.get())
        .on_load(move |info: &VideoInfo| {
            let seconds = f64::from(info.duration);
            transport.length.update(|l| *l = l.max(seconds));
        })
        .on_end(move || transport.paused.set(true));
    let player = if side == 0 {
        player.on_progress(move |p: &VideoProgress| transport.at.set(f64::from(p.current_time)))
    } else {
        player
    };
    viewer_frame().child(player)
}

/// The facts of the two copies, side by side under their panes: each line's
/// name in the middle, the left copy's value toward it from the left, the
/// right copy's from the right. A value that differs is set in ink, one
/// that matches recedes; the better of a pair says so in words.
fn details(
    left: GroupMember,
    right: GroupMember,
    exact: bool,
    facts: Resource<[Facts; 2], ()>,
) -> View {
    view().child(dynamic(move || {
        let Some(read) = facts.get() else {
            return view()
                .h(px(DETAILS_MAX_H))
                .items_center()
                .justify_center()
                .child(caption("Reading the details…"));
        };
        let rows = fact_rows([&left, &right], [&read[0], &read[1]], exact);
        view().child(
            scroll()
                .max_h(px(DETAILS_MAX_H))
                .child(view().flex_col().children(rows.into_iter().map(fact_line))),
        )
    }))
}

/// The tallest the details get before they scroll: the panes keep the rest.
const DETAILS_MAX_H: f32 = 168.0;

/// The width of the details' middle column of names.
const FACT_LABEL_W: f32 = 120.0;

fn fact_line(row: FactRow) -> View {
    let differs = row.differs();
    let cell = |side: usize| {
        let value = row.values[side].clone();
        let shown = if differs {
            amount(value.clone())
        } else {
            description(value.clone())
        }
        .truncate()
        .min_w(px(0.0))
        .tooltip(hint(value));
        let mark = row
            .better
            .filter(|(at, _)| *at == side)
            .map(|(_, word)| badge(word, BadgeTone::Raised));
        let mut line = view()
            .flex_row()
            .flex_1()
            .min_w(px(0.0))
            .items_center()
            .gap(px(space::SM));
        if side == 0 {
            line = line.justify_end();
            if let Some(mark) = mark {
                line = line.child(mark);
            }
            line.child(shown)
        } else {
            line = line.child(shown);
            match mark {
                Some(mark) => line.child(mark),
                None => line,
            }
        }
    };
    view()
        .flex_row()
        .items_center()
        .gap(px(space::MD))
        .py(px(space::XS))
        .child(cell(0))
        .child(
            view()
                .w(px(FACT_LABEL_W))
                .flex_shrink_0()
                .items_center()
                .child(field_label(row.label)),
        )
        .child(cell(1))
}
