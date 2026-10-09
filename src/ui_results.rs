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
use gpui_kit::component::{ActiveTheme, IconName};
use gpui_kit::{
    Context, FontWeight, IntoElement, ObjectFit, ParentElement as _, SharedString, Styled as _,
    StyledImage as _, div, img, px,
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
    let name = member
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| member.path.display().to_string());
    let parent = member
        .path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let mut title = name;
    if let Some(sim) = member.similarity
        && !member.keep
    {
        title.push_str(&format!("  · {:.1}% similar", sim * 100.0));
    }
    if let Some(info) = &member.media {
        title.push_str(&format!("  · {}", info.summary()));
    }

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
