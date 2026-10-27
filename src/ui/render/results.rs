use crate::ui::{DedupeView, GROUP_PAGE, ThemeReadability};
use crate::ui_helpers;
use crate::ui_results::{SnapshotGroup, is_grid_preview, render_media_card, render_member};
use crate::{cli, poster};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::IconName;
use gpui_kit::component::accordion::Accordion;
use gpui_kit::component::alert::Alert;
use gpui_kit::component::button::Button;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tag::Tag;
use gpui_kit::{
    Context, FontWeight, IntoElement, ParentElement as _, Styled as _, div, linear_color_stop,
    linear_gradient, px,
};

impl DedupeView {
    /// First-run empty state: the sidebar holds a dozen options, so the
    /// empty results pane teaches the 3-step flow instead of sitting blank.
    pub(crate) fn render_empty_state(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let duplicates = ui_helpers::find_mode(self.find_index) == cli::FindMode::Duplicates;
        let muted = cx.theme().muted_foreground;
        let border = cx.theme().border;
        let step = |n: &'static str, title: &'static str, text: &'static str, icon: IconName| {
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_2()
                .p_3()
                .rounded(px(12.))
                .border_1()
                .border_color(border)
                .bg(cx.theme().card_surface())
                .shadow_sm()
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .size(px(30.))
                                .rounded(px(9.))
                                .bg(cx.theme().accent.opacity(0.12))
                                .text_color(cx.theme().accent)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon),
                        )
                        .child(
                            div()
                                .text_size(px(cx.theme().text_xs_px()))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(muted)
                                .child(n),
                        ),
                )
                .child(
                    div()
                        .text_size(px(cx.theme().text_sm_px()))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(cx.theme().text_xs_px()))
                        .text_color(muted)
                        .child(text),
                )
        };
        let preview_row = |icon: IconName, label: &'static str, value: String| {
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .size(px(24.))
                        .rounded(px(8.))
                        .bg(cx.theme().muted)
                        .text_color(muted)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon),
                )
                .child(
                    v_flex()
                        .gap_0()
                        .child(
                            div()
                                .text_size(px(cx.theme().text_xs_px()))
                                .text_color(muted)
                                .child(label),
                        )
                        .child(
                            div()
                                .text_size(px(cx.theme().text_sm_px()))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(value),
                        ),
                )
        };
        v_flex()
            .flex_1()
            .w_full()
            .items_center()
            .justify_center()
            .py_4()
            .child(
                v_flex()
                    .w_full()
                    .max_w(px(980.))
                    .gap_5()
                    .p_6()
                    .rounded(px(24.))
                    .border_1()
                    .border_color(border)
                    .bg(linear_gradient(
                        135.,
                        linear_color_stop(cx.theme().background, 0.),
                        linear_color_stop(cx.theme().accent_wash_strong(), 1.),
                    ))
                    .shadow_md()
                    .child(
                        h_flex()
                            .gap_8()
                            .items_center()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_3()
                                    .child(
                                        div()
                                            .text_size(px(cx.theme().text_xs_px()))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(cx.theme().accent)
                                            .child(if duplicates {
                                                "Duplicate scanning"
                                            } else {
                                                "Cleanup scanning"
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(cx.theme().text_heading_px()))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(if duplicates {
                                                "Find every copy. Keep the one you want."
                                            } else {
                                                "Put every stray file back in its place."
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(cx.theme().text_sm_px()))
                                            .text_color(muted)
                                            .child(if duplicates {
                                                "One copy is always kept. Review each group and remove the rest."
                                            } else {
                                                "Scan for empty files, broken media, bad names, and duplicate folders."
                                            }),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .w(px(250.))
                                    .flex_shrink_0()
                                    .gap_3()
                                    .p_4()
                                    .rounded(px(18.))
                                    .border_1()
                                    .border_color(border)
                                    .bg(cx.theme().card_surface())
                                    .shadow_sm()
                                    .child(
                                        h_flex()
                                            .justify_between()
                                            .items_center()
                                            .child(
                                                div()
                                                    .text_size(px(cx.theme().text_sm_px()))
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .child("Scan preview"),
                                            )
                                            .child(Tag::success().child("Ready")),
                                    )
                                    .child(preview_row(
                                        IconName::Folder,
                                        "Folders",
                                        format!("{} queued", self.folders.len()),
                                    ))
                                    .child(preview_row(
                                        IconName::Search,
                                        "Mode",
                                        if duplicates {
                                            "Duplicate scan".to_string()
                                        } else {
                                            "Cleanup scan".to_string()
                                        },
                                    ))
                                    .child(preview_row(
                                        IconName::RefreshCw,
                                        "Matching",
                                        if duplicates {
                                            "Content + media".to_string()
                                        } else {
                                            "Cleanup rules".to_string()
                                        },
                                    ))
                                    .child(preview_row(
                                        IconName::Delete,
                                        "Delete",
                                        if self.trash {
                                            "To trash".to_string()
                                        } else {
                                            "Permanently".to_string()
                                        },
                                    ))
                                    .child(
                                        div()
                                            .h(px(4.))
                                            .w_full()
                                            .rounded_full()
                                            .bg(cx.theme().accent.opacity(0.14))
                                            .child(
                                                div()
                                                    .h(px(4.))
                                                    .w(px(96.))
                                                    .rounded_full()
                                                    .bg(cx.theme().accent),
                                            ),
                                    ),
                            ),
                    )
                    .child(div().h(px(1.)).w_full().bg(border))
                    .child(
                        h_flex()
                            .items_stretch()
                            .gap_3()
                            .child(step(
                                "01",
                                "Add folders",
                                "Choose the folders you want to scan.",
                                IconName::Folder,
                            ))
                            .child(step(
                                "02",
                                "Run a scan",
                                if duplicates {
                                    "Files, photos, and videos are compared on this PC."
                                } else {
                                    "Cleanup rules check for specific problems."
                                },
                                IconName::Search,
                            ))
                            .child(step(
                                "03",
                                "Review results",
                                if duplicates {
                                    "Keep the right copy and remove the rest."
                                } else {
                                    "Inspect every finding before using the CLI."
                                },
                                IconName::Check,
                            )),
                    ),
            )
    }
    pub(crate) fn render_results(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        if self.mode_result.is_some() {
            return self.render_mode_results(cx).into_any_element();
        }
        // Two rows: expand/collapse + select-all on top, all action
        // controls on the second row right-aligned. The scan status lives
        // in the status bar (and the progress card while scanning), so the
        // header never repeats it. Flex items will not shrink below
        // content width here, so controls must never share a row with
        // free-length text — otherwise they clip off the edge.
        let title = if self.has_scanned {
            "Review duplicates"
        } else {
            "Find duplicate files"
        };
        let subtitle = if self.scanning {
            "Scanning. Results will appear here."
        } else if self.has_scanned {
            "Keep one copy. Select the rest."
        } else {
            "Add folders and scan. Nothing is uploaded."
        };
        let state = if self.scanning {
            "Running".to_string()
        } else if self.has_scanned {
            format!("{} groups", self.groups.len())
        } else {
            "Ready".to_string()
        };
        let header = v_flex()
            .gap_3()
            .child(
                h_flex()
                    .justify_between()
                    .items_end()
                    .gap_4()
                    .child(
                        v_flex()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_size(px(cx.theme().text_xs_px()))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(cx.theme().accent)
                                    .child("Duplicate workspace"),
                            )
                            .child(
                                div()
                                    .text_size(px(cx.theme().text_heading_px()))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(title),
                            )
                            .child(
                                div()
                                    .text_size(px(cx.theme().text_sm_px()))
                                    .text_color(cx.theme().muted_foreground)
                                    .child(subtitle),
                            ),
                    )
                    .child(Tag::secondary().child(state)),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(self.render_expand_toggle(cx))
                    .child(self.render_select_presets(cx)),
            )
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .justify_end()
                    .child(self.render_sim_filter(cx))
                    .child(self.render_keep_segmented(cx))
                    .child(self.render_sort_toggle(cx))
                    .child(self.render_delete_bar(cx)),
            );
        let mut list = v_flex().gap_2();
        if self.scanning {
            list = list.child(self.render_progress(cx));
        }
        if !self.has_scanned && !self.scanning {
            list = list.child(self.render_empty_state(cx));
        }
        if self.has_scanned && !self.groups.is_empty() {
            list = list.child(self.render_stat_chips(cx));
        }
        if self.ffprobe_note {
            list = list.child(Alert::warning(
                "ffprobe-note",
                "ffprobe not found on PATH — install FFmpeg for media resolution/duration.",
            ));
        }
        if self.has_scanned && self.groups.is_empty() {
            list = list.child(Alert::success("no-dups", "No duplicate files found."));
        }
        // Render in pages: thousands of groups would otherwise build
        // thousands of elements up front and stall the UI. Order comes
        // from sort_groups (scan / toggle / filter events only) — rendering
        // itself never re-sorts, so keeper picks can't move groups around.
        // All groups are ONE kit Accordion (bordered card, per-item
        // chevron + animated reveal). The accordion reports its open set
        // as positions; mirror it back to group indices so Expand all /
        // Collapse all keep driving `expanded`.
        let total = self.groups.len();
        if total > 0 {
            let view = cx.entity();
            let mut acc = Accordion::new("groups")
                .multiple(true)
                .w_full()
                .on_toggle_click(move |open: &[usize], _, cx| {
                    view.update(cx, |this, cx| {
                        this.expanded = open
                            .iter()
                            .filter_map(|&pos| this.groups.get(pos).map(|g| g.index))
                            .collect();
                        cx.notify();
                    });
                });
            let header_size = cx.theme().text_sm_px();
            for gi in 0..total.min(self.group_limit) {
                let expanded = self.expanded.contains(&self.groups[gi].index);
                acc = acc.item(|item| {
                    let item = item
                        .title(self.group_header(gi, header_size))
                        .open(expanded);
                    if expanded {
                        item.child(self.group_body(gi, cx))
                    } else {
                        item
                    }
                });
            }
            list = list.child(acc);
        }
        if total > self.group_limit {
            let remaining = total - self.group_limit;
            list = list.child(
                Button::new("show-more-groups")
                    .icon(IconName::ChevronDown)
                    .label(format!("Show more ({remaining} remaining)"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.group_limit += GROUP_PAGE;
                        cx.notify();
                    })),
            );
        }
        v_flex()
            .flex_1()
            .h_full()
            .gap_4()
            .p_6()
            .bg(linear_gradient(
                145.,
                linear_color_stop(cx.theme().background, 0.),
                linear_color_stop(cx.theme().accent_wash(), 1.),
            ))
            .child(header)
            .child(div().flex_1().w_full().overflow_y_scrollbar().child(list))
            .into_any_element()
    }
    /// Group header for the accordion trigger. The kit trigger draws the
    /// rotating chevron and the spring reveal, so no glyphs live here.
    pub(crate) fn group_header(&self, gi: usize, font_size: f32) -> impl IntoElement {
        let snapshot = self.groups[gi].clone_snapshot();
        div()
            .text_size(px(font_size))
            .truncate()
            .child(snapshot.header)
    }
    /// Expanded group body: the media card grid (when every member has a
    /// large preview) or plain member rows, plus the lazy inline
    /// video-strip extraction kickoff. Rendered only for expanded groups.
    pub(crate) fn group_body(&mut self, gi: usize, cx: &mut Context<Self>) -> impl IntoElement {
        // Snapshot the group first: click handlers must own their data
        // ('static), so nothing borrowed from `self` may leak into them.
        let snapshot = self.groups[gi].clone_snapshot();
        let mut body = v_flex().gap_1();
        // Single-display grid for media groups: every member has a
        // large visual (image or video poster), so render each once as a
        // side-by-side card (left card / right card, info under each)
        // instead of big-preview + duplicate rows. Keeper cards get a
        // highlight border; clicking a preview promotes it to keeper.
        let thumbs: Vec<crate::ui_results::Thumb> =
            snapshot.members.iter().map(|m| self.thumb_for(m)).collect();
        let grid = snapshot.members.len() >= 2 && thumbs.iter().all(is_grid_preview);
        if grid {
            for chunk in snapshot.members.chunks(2).enumerate() {
                let (ci, members) = chunk;
                // Stretch keeps both cards in a row the same height so
                // images and videos always display at the same size.
                let mut row = h_flex().gap_2().pl_6().items_stretch();
                for (offset, member) in members.iter().enumerate() {
                    let mi = ci * 2 + offset;
                    let checked = self.selected.contains(&member.path);
                    let thumb = self.thumb_for(member);
                    // Lazy inline strip: one background extraction per
                    // video, kicked off on first sight (render may run
                    // often; the loading set + disk cache dedupe it).
                    if matches!(
                        thumb,
                        crate::ui_results::Thumb::Poster(_)
                            | crate::ui_results::Thumb::PendingVideo
                    ) && !self.strips.contains_key(&member.path)
                        && !self.preview_loading.contains(&member.path)
                    {
                        self.preview_loading.insert(member.path.clone());
                        let job_path = member.path.clone();
                        let done_path = member.path.clone();
                        let size = member.size;
                        let mtime = member.mtime_secs;
                        cx.spawn(async move |this, cx| {
                                let frames = cx
                                    .background_executor()
                                    .spawn(async move {
                                        poster::video_strip(
                                            &job_path,
                                            size,
                                            mtime,
                                            poster::STRIP_FRAMES,
                                        )
                                    })
                                    .await;
                                this.update(cx, |view, cx| {
                                    view.preview_loading.remove(&done_path);
                                    if frames.is_empty() {
                                        view.status = "Inline preview unavailable — install ffmpeg or check the file."
                                            .to_string();
                                    } else {
                                        view.strips.insert(done_path, frames);
                                    }
                                    cx.notify();
                                })
                                .ok();
                            })
                            .detach();
                    }
                    let video = crate::ui_results::VideoPreview {
                        strip: self.strips.get(&member.path).cloned(),
                        loading: self.preview_loading.contains(&member.path),
                    };
                    row = row.child(render_media_card(
                        snapshot.index,
                        mi,
                        member,
                        checked,
                        &thumb,
                        &video,
                        cx,
                    ));
                }
                // Odd tail: keep the single card left-aligned at half
                // width instead of stretching full width.
                if members.len() == 1 {
                    row = row.child(div().flex_1());
                }
                body = body.child(row);
            }
        } else {
            for (mi, member) in snapshot.members.iter().enumerate() {
                let checked = self.selected.contains(&member.path);
                let thumb = self.thumb_for(member);
                body = body.child(render_member(
                    snapshot.index,
                    mi,
                    member,
                    checked,
                    &thumb,
                    cx,
                ));
            }
        }
        body
    }
}
