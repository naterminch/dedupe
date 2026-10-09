//! Results-pane rendering extracted from `ui.rs`: per-group snapshots,
//! member rows, and thumbnail previews.
//!
//! The view state stays in `ui.rs`; this module owns the pure-ish row UI.
//! Anything reaching into `DedupeView` goes through `pub(crate)` access.

use crate::ui::{DedupeView, THUMB_EXTS};
use crate::ui_helpers::fit_text;
use crate::{media, report, util};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme, IconName, WindowExt};
use gpui_kit::{
    Context, FontWeight, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, SharedString, StatefulInteractiveElement as _, Styled as _,
    StyledImage as _, Window, div, img, px,
};
use std::path::PathBuf;

/// Owned per-group data for rendering. Groups are replaced wholesale on
/// every scan/delete, so click handlers capture this snapshot instead of
/// borrowing the view (handlers must be `'static`).
pub(crate) struct GroupSnapshot {
    pub(crate) index: usize,
    pub(crate) header: String,
    pub(crate) members: Vec<crate::matching::GroupMember>,
}

pub(crate) trait SnapshotGroup {
    fn clone_snapshot(&self) -> GroupSnapshot;
}

impl SnapshotGroup for crate::matching::Group {
    fn clone_snapshot(&self) -> GroupSnapshot {
        // Exact groups omit the content hash: identical bytes are implied
        // by the grouping itself, and a hex prefix is not actionable.
        // Similar groups keep their score — that one informs the decision.
        let mut parts = vec![
            format!("Group #{}", self.index),
            report::group_kind_name(self.media_kind).to_uppercase(),
            format!("{} files", self.members.len()),
            format!("{} reclaimable", util::human_bytes(self.dup_bytes())),
        ];
        if let Some(s) = self.similarity {
            parts.push(format!("{:.1}% similar", s * 100.0));
        }
        if let Some(summary) = self
            .members
            .first()
            .and_then(|m| m.media.clone())
            .map(|info| info.summary())
        {
            parts.push(summary);
        }
        GroupSnapshot {
            index: self.index,
            header: parts.join(" · "),
            members: self.members.clone(),
        }
    }
}

impl GroupSnapshot {
    pub(crate) fn header_label(&self, expanded: bool) -> String {
        format!("{} {}", if expanded { "▾ " } else { "▸ " }, self.header)
    }
}

/// What preview a member row shows: none, the image itself, a generated
/// video poster, or a placeholder while the poster extracts.
pub(crate) enum Thumb {
    None,
    Image,
    Poster(PathBuf),
    PendingVideo,
}

impl DedupeView {
    /// Decide what preview a member row shows.
    pub(crate) fn thumb_for(&self, member: &crate::matching::GroupMember) -> Thumb {
        match media::classify(&member.path) {
            media::MediaKind::Image => {
                let thumbable = member
                    .path
                    .extension()
                    .and_then(|e| e.to_str())
                    .map(|e| THUMB_EXTS.contains(&e.to_ascii_lowercase().as_str()))
                    .unwrap_or(false);
                if thumbable {
                    Thumb::Image
                } else {
                    Thumb::None
                }
            }
            media::MediaKind::Video => match self.posters.get(&member.path) {
                Some(p) => Thumb::Poster(p.clone()),
                None => Thumb::PendingVideo,
            },
            media::MediaKind::Other => Thumb::None,
        }
    }
}

/// Thumbnail box for a member row: the image itself, a video poster, a
/// placeholder while the poster extracts, or nothing for plain files.
pub(crate) fn render_thumb(
    path: &std::path::Path,
    thumb: &Thumb,
    cx: &mut Context<DedupeView>,
) -> impl IntoElement {
    match thumb {
        Thumb::None => div().into_any_element(),
        Thumb::Image => fixed_thumb(img(path)),
        Thumb::Poster(poster) => fixed_thumb(img(poster.clone())),
        Thumb::PendingVideo => div()
            .w(px(72.))
            .h(px(56.))
            .rounded_md()
            .bg(cx.theme().secondary)
            .flex()
            .items_center()
            .justify_center()
            .child(IconName::Play)
            .into_any_element(),
    }
}

/// Fixed 72×56 thumbnail box (cover-fit): async image loads must never
/// change row geometry, or rows overlap mid-layout.
fn fixed_thumb(image: gpui_kit::Img) -> gpui_kit::AnyElement {
    image
        .w(px(72.))
        .h(px(56.))
        .object_fit(ObjectFit::Cover)
        .rounded_md()
        .into_any_element()
}

pub(crate) fn render_member(
    group_index: usize,
    mi: usize,
    member: &crate::matching::GroupMember,
    checked: bool,
    thumb: &Thumb,
    cx: &mut Context<DedupeView>,
) -> impl IntoElement {
    // Three-line row: the file name gets top billing, the parent folder
    // sits dimmed underneath, and all fixed-size controls share a third
    // line. Flex items will not shrink below content width here, so text
    // must never share a row with buttons.
    let parent = member
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let title = member_title(member);

    let mut actions = h_flex().gap_2().items_center();
    if member.keep {
        let (badge, color) = if member.reference {
            ("◈ REF", cx.theme().accent)
        } else {
            ("✓ KEEP", cx.theme().success)
        };
        actions = actions.child(
            div()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(color)
                .child(badge),
        );
    } else {
        let key = member.path.clone();
        actions = actions
            .child(
                Checkbox::new(SharedString::from(format!("dup-{group_index}-{mi}")))
                    .checked(checked)
                    .on_change(cx.listener(move |this, value, _, cx| {
                        if *value {
                            this.selected.insert(key.clone());
                        } else {
                            this.selected.remove(&key);
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(cx.theme().warning)
                    .child("DUP"),
            );
    }
    actions = actions.child(
        div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(util::human_bytes(member.size)),
    );
    if !member.keep {
        let keep_path = member.path.clone();
        actions = actions.child(
            Button::new(SharedString::from(format!("keep-{group_index}-{mi}")))
                .label("Keep")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_keeper(group_index, &keep_path, cx);
                })),
        );
    }
    if matches!(thumb, Thumb::Poster(_) | Thumb::PendingVideo) {
        let play_path = member.path.clone();
        actions = actions.child(
            Button::new(SharedString::from(format!("play-{group_index}-{mi}")))
                .icon(IconName::Play)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Err(e) = open::that(&play_path) {
                        this.status = format!("Could not play file: {e}");
                        cx.notify();
                    }
                })),
        );
    }
    {
        let reveal = member
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| member.path.clone());
        actions = actions.child(
            Button::new(SharedString::from(format!("reveal-{group_index}-{mi}")))
                .icon(IconName::FolderOpen)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Err(e) = open::that(&reveal) {
                        this.status = format!("Could not open folder: {e}");
                        cx.notify();
                    }
                })),
        );
    }

    h_flex()
        .gap_2()
        .items_start()
        .pl_6()
        .child(render_thumb(&member.path, thumb, cx))
        .child(
            v_flex()
                .flex_1()
                .gap_1()
                .child(div().text_sm().truncate().child(fit_text(&title, 120)))
                .child(
                    div()
                        .text_xs()
                        .truncate()
                        .text_color(cx.theme().muted_foreground)
                        .child(fit_text(&parent, 160)),
                )
                .child(actions),
        )
}

/// True when a member has a large previewable visual (image file or video
/// poster/placeholder). Groups where every member qualifies render as a
/// side-by-side card grid instead of the vertical row list.
pub(crate) fn is_grid_preview(thumb: &Thumb) -> bool {
    matches!(
        thumb,
        Thumb::Image | Thumb::Poster(_) | Thumb::PendingVideo
    )
}

/// Card/row/modal title: identical for keeper and dups (name, similarity,
/// media summary), so flipping the keeper only moves the KEEP/DUP badge
/// and border, never the text.
pub(crate) fn member_title(member: &crate::matching::GroupMember) -> String {
    let mut title = member
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| member.path.display().to_string());
    if let Some(sim) = member.similarity {
        title.push_str(&format!("  · {:.1}% similar", sim * 100.0));
    }
    if let Some(info) = &member.media {
        title.push_str(&format!("  · {}", info.summary()));
    }
    title
}

/// Inline-preview state for a video card: cached frame strip plus whether
/// the strip is still extracting. The strip always displays (no toggle).
pub(crate) struct VideoPreview {
    pub(crate) strip: Option<Vec<PathBuf>>,
    pub(crate) loading: bool,
}

/// One media card: large preview on top (click to keep), inline video
/// strip when expanded, info + actions below. The keeper gets a
/// success/accent border; others stay neutral. Card order never changes —
/// selecting a keeper only moves the highlight and the KEEP/DUP labels.
pub(crate) fn render_media_card(
    group_index: usize,
    mi: usize,
    member: &crate::matching::GroupMember,
    checked: bool,
    thumb: &Thumb,
    video: VideoPreview,
    cx: &mut Context<DedupeView>,
) -> impl IntoElement {
    let parent = member
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let title = member_title(member);

    let mut actions = h_flex().gap_2().items_center().flex_wrap().w_full();
    if member.keep {
        let (badge, color) = if member.reference {
            ("◈ REF", cx.theme().accent)
        } else {
            ("✓ KEEP", cx.theme().success)
        };
        actions = actions.child(
            div()
                .text_xs()
                .font_weight(FontWeight::BOLD)
                .text_color(color)
                .child(badge),
        );
    } else {
        let key = member.path.clone();
        actions = actions
            .child(
                Checkbox::new(SharedString::from(format!("dup-{group_index}-{mi}")))
                    .checked(checked)
                    .on_change(cx.listener(move |this, value, _, cx| {
                        if *value {
                            this.selected.insert(key.clone());
                        } else {
                            this.selected.remove(&key);
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .text_color(cx.theme().warning)
                    .child("DUP"),
            );
    }
    actions = actions.child(
        div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(util::human_bytes(member.size)),
    );
    if !member.keep {
        let keep_path = member.path.clone();
        actions = actions.child(
            Button::new(SharedString::from(format!("keep-{group_index}-{mi}")))
                .label("Keep")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_keeper(group_index, &keep_path, cx);
                })),
        );
    }
    // Videos always show their inline frame strip below the main preview
    // (extraction kicks off in render_group on first sight).
    let is_video = matches!(thumb, Thumb::Poster(_) | Thumb::PendingVideo);
    {
        let reveal = member
            .path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| member.path.clone());
        actions = actions.child(
            Button::new(SharedString::from(format!("reveal-{group_index}-{mi}")))
                .icon(IconName::FolderOpen)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Err(e) = open::that(&reveal) {
                        this.status = format!("Could not open folder: {e}");
                        cx.notify();
                    }
                })),
        );
    }

    let (border, bg) = if member.keep {
        let c = if member.reference {
            cx.theme().accent
        } else {
            cx.theme().success
        };
        (c, cx.theme().background)
    } else {
        (cx.theme().border, cx.theme().background)
    };

    // Preview clicks: single click promotes to keeper (remembering the
    // previous one); double-click reverts that flip — "just looking" never
    // moves KEEP — and opens the gallery modal with every image/video in
    // the group, large. The id + handler go directly on the image itself
    // (Img is interactive), so keeper and dup previews keep the exact same
    // layout box. A wrapping stateful div around the image breaks its
    // sizing and spills the photo over the group.
    let preview: gpui_kit::AnyElement = {
        let id = SharedString::from(format!("pick-{group_index}-{mi}"));
        // One handler shape per arm (closures don't clone): single = keep,
        // double = revert + gallery. Keeper previews get it too, so
        // double-clicking the keeper also opens the gallery.
        match thumb {
            Thumb::Image => {
                let keep_path = member.path.clone();
                img(member.path.clone())
                    .h(px(180.))
                    .w_full()
                    .object_fit(ObjectFit::Cover)
                    .rounded_md()
                    .id(id)
                    .cursor_pointer()
                    .on_click(cx.listener(
                        move |this: &mut DedupeView,
                              ev: &gpui_kit::ClickEvent,
                              window: &mut Window,
                              cx| {
                            if ev.click_count() == 1 {
                                if let Some(g) =
                                    this.groups.iter().find(|g| g.index == group_index)
                                {
                                    this.last_pick = g
                                        .members
                                        .iter()
                                        .find(|m| m.keep)
                                        .map(|m| (group_index, m.path.clone()));
                                }
                                this.set_keeper(group_index, &keep_path, cx);
                            } else if ev.click_count() == 2 {
                                if let Some((gi, prev)) = this.last_pick.take()
                                    && gi == group_index
                                {
                                    this.set_keeper(group_index, prev.as_path(), cx);
                                }
                                this.open_gallery(group_index, window, cx);
                            }
                        },
                    ))
                    .into_any_element()
            }
            Thumb::Poster(poster) => {
                let keep_path = member.path.clone();
                img(poster.clone())
                    .h(px(180.))
                    .w_full()
                    .object_fit(ObjectFit::Cover)
                    .rounded_md()
                    .id(id)
                    .cursor_pointer()
                    .on_click(cx.listener(
                        move |this: &mut DedupeView,
                              ev: &gpui_kit::ClickEvent,
                              window: &mut Window,
                              cx| {
                            if ev.click_count() == 1 {
                                if let Some(g) =
                                    this.groups.iter().find(|g| g.index == group_index)
                                {
                                    this.last_pick = g
                                        .members
                                        .iter()
                                        .find(|m| m.keep)
                                        .map(|m| (group_index, m.path.clone()));
                                }
                                this.set_keeper(group_index, &keep_path, cx);
                            } else if ev.click_count() == 2 {
                                if let Some((gi, prev)) = this.last_pick.take()
                                    && gi == group_index
                                {
                                    this.set_keeper(group_index, prev.as_path(), cx);
                                }
                                this.open_gallery(group_index, window, cx);
                            }
                        },
                    ))
                    .into_any_element()
            }
            Thumb::PendingVideo => {
                let keep_path = member.path.clone();
                div()
                    .id(id)
                    .cursor_pointer()
                    .h(px(180.))
                    .w_full()
                    .rounded_md()
                    .bg(cx.theme().secondary)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(IconName::Play)
                    .on_click(cx.listener(
                        move |this: &mut DedupeView,
                              ev: &gpui_kit::ClickEvent,
                              window: &mut Window,
                              cx| {
                            if ev.click_count() == 1 {
                                if let Some(g) =
                                    this.groups.iter().find(|g| g.index == group_index)
                                {
                                    this.last_pick = g
                                        .members
                                        .iter()
                                        .find(|m| m.keep)
                                        .map(|m| (group_index, m.path.clone()));
                                }
                                this.set_keeper(group_index, &keep_path, cx);
                            } else if ev.click_count() == 2 {
                                if let Some((gi, prev)) = this.last_pick.take()
                                    && gi == group_index
                                {
                                    this.set_keeper(group_index, prev.as_path(), cx);
                                }
                                this.open_gallery(group_index, window, cx);
                            }
                        },
                    ))
                    .into_any_element()
            }
            Thumb::None => div().into_any_element(),
        }
    };

    let mut card = v_flex()
        .flex_1()
        .gap_2()
        .p_2()
        .border_2()
        .rounded_md()
        .border_color(border)
        .bg(bg)
        .child(preview);
    // Inline video strip: always visible, same-width frame row under the
    // main preview. Frames share the row equally via flex directly on the
    // images (no wrapper divs — those break image sizing).
    if is_video {
        match &video.strip {
            Some(frames) if !frames.is_empty() => {
                let mut row = h_flex().gap_1().w_full();
                for f in frames {
                    row = row.child(
                        img(f.clone())
                            .h(px(64.))
                            .flex_1()
                            .object_fit(ObjectFit::Cover)
                            .rounded_md(),
                    );
                }
                card = card.child(row);
            }
            _ if video.loading => {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("Loading preview…"),
                );
            }
            _ => {
                card = card.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().warning)
                        .child("Preview unavailable — install ffmpeg."),
                );
            }
        }
    }
    card
        .child(
            div()
                .w_full()
                .text_sm()
                .truncate()
                .child(fit_text(&title, 60)),
        )
        .child(
            div()
                .w_full()
                .text_xs()
                .truncate()
                .text_color(cx.theme().muted_foreground)
                .child(fit_text(&parent, 80)),
        )
        .child(actions)
}

impl DedupeView {
    /// Gallery modal: every image/video in the group, large and uncropped
    /// (Contain), with Keep picks. Launched by double-clicking a card
    /// preview. Keep buttons refresh the modal in place (close + reopen),
    /// same as the delete-confirm dialog.
    pub(crate) fn open_gallery(
        &mut self,
        group_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(g) = self.groups.iter().find(|g| g.index == group_index).cloned() else {
            return;
        };
        // Owned snapshots: the dialog closure must be 'static.
        let items: Vec<(
            crate::matching::GroupMember,
            Thumb,
            Vec<PathBuf>,
        )> = g
            .members
            .iter()
            .map(|m| {
                let thumb = self.thumb_for(m);
                let strip = self.strips.get(&m.path).cloned().unwrap_or_default();
                (m.clone(), thumb, strip)
            })
            .collect();
        let gi = g.index;
        let title = format!(
            "Group #{} · {} · {} file(s)",
            gi,
            report::group_kind_name(g.media_kind).to_uppercase(),
            g.members.len()
        );
        // Theme snapshot: the dialog builder only gets `&mut App`, so grab
        // every color here while the view Context is available.
        let c_secondary = cx.theme().secondary;
        let c_muted = cx.theme().muted_foreground;
        let c_accent = cx.theme().accent;
        let c_success = cx.theme().success;
        let c_warning = cx.theme().warning;
        let view = cx.entity();
        window.open_dialog(cx, move |dialog, _, _| {
            let gallery_view = view.clone();
            // Horizontal filmstrip: items flow left-to-right at a fixed
            // width and wrap onto more rows as needed.
            let mut stack = h_flex().gap_3().flex_wrap().w_full();
            for (mi, (member, thumb, strip)) in items.iter().enumerate() {
                let parent = member
                    .path
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default();
                // Large + uncropped: comparison needs the whole frame, not
                // the cover-cropped card thumbnail.
                let big: gpui_kit::AnyElement = match thumb {
                    Thumb::Image => img(member.path.clone())
                        .h(px(300.))
                        .w_full()
                        .object_fit(ObjectFit::Contain)
                        .rounded_md()
                        .into_any_element(),
                    Thumb::Poster(poster) => img(poster.clone())
                        .h(px(300.))
                        .w_full()
                        .object_fit(ObjectFit::Contain)
                        .rounded_md()
                        .into_any_element(),
                    _ => div()
                        .h(px(300.))
                        .w_full()
                        .rounded_md()
                        .bg(c_secondary)
                        .into_any_element(),
                };
                let mut block = v_flex()
                    .w(px(320.))
                    .flex_none()
                    .gap_1()
                    .child(big)
                    .child(div().text_sm().child(fit_text(&member_title(member), 100)))
                    .child(
                        div()
                            .text_xs()
                            .text_color(c_muted)
                            .child(fit_text(&parent, 120)),
                    );
                if !strip.is_empty() {
                    let mut row = h_flex().gap_1().w_full();
                    for f in strip {
                        row = row.child(
                            img(f.clone())
                                .h(px(64.))
                                .flex_1()
                                .object_fit(ObjectFit::Cover)
                                .rounded_md(),
                        );
                    }
                    block = block.child(row);
                }
                block = block.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(if member.keep {
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(if member.reference {
                                    c_accent
                                } else {
                                    c_success
                                })
                                .child(if member.reference {
                                    "◈ REF KEEPER"
                                } else {
                                    "✓ KEEPER"
                                })
                                .into_any_element()
                        } else {
                            div()
                                .text_xs()
                                .font_weight(FontWeight::BOLD)
                                .text_color(c_warning)
                                .child("DUP")
                                .into_any_element()
                        })
                        .child(
                            div()
                                .text_xs()
                                .text_color(c_muted)
                                .child(util::human_bytes(member.size)),
                        )
                        .child(if member.keep {
                            div().into_any_element()
                        } else {
                            let keep_path = member.path.clone();
                            let reopen = gallery_view.clone();
                            Button::new(SharedString::from(format!(
                                "gallery-keep-{gi}-{mi}"
                            )))
                            .label("Keep this one")
                            .on_click(move |_, window, cx| {
                                reopen.update(cx, |v, cx| {
                                    v.set_keeper(gi, &keep_path, cx);
                                });
                                window.close_dialog(cx);
                                let again = reopen.clone();
                                again.update(cx, |v, cx| {
                                    v.open_gallery(gi, window, cx);
                                });
                            })
                            .into_any_element()
                        }),
                );
                stack = stack.child(block);
            }
            // Tall groups scroll inside a fixed-height body; small groups
            // size naturally.
            let body: gpui_kit::AnyElement = if items.len() > 3 {
                div()
                    .h(px(560.))
                    .overflow_y_scrollbar()
                    .child(stack)
                    .into_any_element()
            } else {
                stack.into_any_element()
            };
            dialog
                .title(title.clone())
                .width(px(720.))
                .child(body)
                .footer(
                    h_flex().gap_2().justify_end().child(
                        Button::new("gallery-close").label("Close").on_click(
                            |_, window, cx| {
                                window.close_dialog(cx);
                            },
                        ),
                    ),
                )
        });
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::matching::{Group, GroupMember};
    use crate::media::MediaKind;

    fn member(path: &str, keep: bool, similarity: Option<f64>) -> GroupMember {
        GroupMember {
            path: PathBuf::from(path),
            size: 100,
            mtime_secs: None,
            keep,
            reference: false,
            media: None,
            similarity,
            content_hash: None,
            fingerprint_res: None,
        }
    }

    #[test]
    fn exact_snapshot_omits_hash_and_keeps_score_out() {
        let g = Group {
            index: 3,
            hash: "deadbeefcafe".to_string(),
            media_kind: MediaKind::Other,
            similarity: None,
            members: vec![member("k.txt", true, None), member("d.txt", false, None)],
        };
        let snap = g.clone_snapshot();
        assert_eq!(snap.index, 3);
        assert!(snap.header.contains("Group #3"));
        assert!(snap.header.contains("FILE"));
        assert!(!snap.header.contains("deadbeef"), "hash is not actionable");
        assert!(!snap.header.contains("similar"));
        assert_eq!(snap.header_label(true).chars().next().unwrap(), '▾');
        assert_eq!(snap.header_label(false).chars().next().unwrap(), '▸');
    }

    #[test]
    fn similar_snapshot_shows_score_and_reclaimable() {
        let g = Group {
            index: 1,
            hash: "img".to_string(),
            media_kind: MediaKind::Image,
            similarity: Some(0.973),
            members: vec![
                member("k.png", true, None),
                member("d.png", false, Some(0.973)),
            ],
        };
        let snap = g.clone_snapshot();
        assert!(snap.header.contains("97.3% similar"), "{}", snap.header);
        assert!(snap.header.contains("IMAGE"), "{}", snap.header);
        assert!(snap.header.contains("reclaimable"), "{}", snap.header);
    }
}
