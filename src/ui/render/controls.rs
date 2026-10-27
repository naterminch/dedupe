use crate::ui::{DedupeView, ThemeReadability};
use crate::util;
use gpui_kit::base::{Disableable as _, Selectable as _, h_flex, v_flex};
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::IconName;
use gpui_kit::component::button::{Button, ButtonGroup, DropdownButton};
use gpui_kit::component::group_box::GroupBox;
use gpui_kit::component::input::NumberInput;
use gpui_kit::component::label::Label;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::{
    Context, FontWeight, IntoElement, ParentElement as _, SharedString, Styled as _, div, px,
};
use std::collections::HashSet;
use std::path::PathBuf;

impl DedupeView {
    /// Live scan progress card: phase label with "N of M" counts plus a
    /// real progress bar for the determinate hashing/comparing stages.
    pub(crate) fn render_progress(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut heading = h_flex().gap_2().items_center();
        if !self.scan_determinate {
            heading = heading.child(Spinner::new().color(cx.theme().accent));
        }
        let title_row = h_flex()
            .items_center()
            .justify_between()
            .gap_2()
            .child(
                heading.child(
                    div()
                        .text_size(px(cx.theme().text_sm_px()))
                        .font_weight(FontWeight::BOLD)
                        .text_color(cx.theme().accent)
                        .child(self.status.clone()),
                ),
            )
            .child(
                div()
                    .text_size(px(cx.theme().text_xs_px()))
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{} file(s) scanned", self.files_scanned)),
            );
        let mut card = GroupBox::new().id("scan-progress").title(title_row);
        if self.scan_determinate {
            let pct = if self.scan_total > 0 {
                (self.scan_done as f32 / self.scan_total as f32 * 100.0).clamp(0.0, 100.0)
            } else {
                0.0
            };
            card = card.child(Progress::new("scan-progress-bar").value(pct).w_full());
        }
        card
    }
    /// Summary chips: files scanned, groups found, reclaimable bytes.
    pub(crate) fn render_stat_chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let chip = |icon: IconName, value: String, label: &'static str, color: gpui_kit::Hsla| {
            h_flex()
                .flex_1()
                .min_w_0()
                .gap_3()
                .items_center()
                .p_3()
                .border_1()
                .rounded(px(12.))
                .border_color(cx.theme().border)
                .bg(cx.theme().card_surface())
                .shadow_sm()
                .child(
                    div()
                        .size(px(34.))
                        .rounded(px(10.))
                        .bg(color.opacity(0.12))
                        .text_color(color)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon),
                )
                .child(
                    v_flex()
                        .min_w_0()
                        .gap_0()
                        .child(
                            div()
                                .text_size(px(cx.theme().text_sm_px() + 3.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(color)
                                .child(value),
                        )
                        .child(
                            div()
                                .text_size(px(cx.theme().text_xs_px()))
                                .text_color(cx.theme().muted_foreground)
                                .child(label),
                        ),
                )
        };
        h_flex()
            .w_full()
            .gap_2()
            .child(chip(
                IconName::Search,
                self.files_scanned.to_string(),
                "Files scanned",
                cx.theme().foreground,
            ))
            .child(chip(
                IconName::Folder,
                self.groups.len().to_string(),
                "Groups",
                cx.theme().accent,
            ))
            .child(chip(
                IconName::Delete,
                util::human_bytes(self.reclaim_bytes),
                "Reclaimable",
                cx.theme().success,
            ))
    }
    /// Every non-keeper path: the universe bulk selection operates on.
    /// Keepers are never selectable.
    pub(crate) fn all_dup_paths(&self) -> HashSet<PathBuf> {
        self.groups
            .iter()
            .flat_map(|g| g.members.iter())
            .filter(|m| !m.keep)
            .map(|m| m.path.clone())
            .collect()
    }
    /// Bulk selection presets: All / None / Invert. Keepers stay untouched.
    pub(crate) fn render_select_presets(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let off = self.scanning || self.groups.is_empty();
        h_flex()
            .gap_1()
            .items_center()
            .child(Label::new("Select"))
            .child(
                Button::new("sel-all")
                    .label("All")
                    .disabled(off)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selected = this.all_dup_paths();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("sel-none")
                    .label("None")
                    .disabled(off || self.selected.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.selected.clear();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("sel-invert")
                    .label("Invert")
                    .disabled(off)
                    .on_click(cx.listener(|this, _, _, cx| {
                        let all = this.all_dup_paths();
                        this.selected = all.difference(&this.selected).cloned().collect();
                        cx.notify();
                    })),
            )
    }
    /// Bulk expand/collapse for the group list.
    pub(crate) fn render_expand_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let off = self.scanning || self.groups.is_empty();
        h_flex()
            .gap_1()
            .items_center()
            .child(
                Button::new("expand-all")
                    .label("Expand all")
                    .disabled(off)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.expanded = this.groups.iter().map(|g| g.index).collect();
                        cx.notify();
                    })),
            )
            .child(
                Button::new("collapse-all")
                    .label("Collapse all")
                    .disabled(off || self.expanded.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.expanded.clear();
                        cx.notify();
                    })),
            )
    }
    /// Live similarity floor: type a higher % to tighten the current
    /// results without rescanning (blank restores the scan floor).
    /// Shown only when similar-media groups exist; exact groups always
    /// qualify and are unaffected.
    pub(crate) fn render_sim_filter(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_sim = self.groups.iter().any(|g| g.similarity.is_some());
        if self.scanning || self.groups.is_empty() || !has_sim {
            return div().into_any_element();
        }
        h_flex()
            .gap_1()
            .items_center()
            .child(Label::new("Similarity floor"))
            .child(
                div().w(px(160.)).child(
                    NumberInput::new(&self.min_sim_input)
                        .suffix(div().text_size(px(cx.theme().text_xs_px())).child("%")),
                ),
            )
            .into_any_element()
    }
    /// Global keep mode as a segmented control (First / Smallest / Newest /
    /// Oldest / Best quality); reassigns keepers on the current results, no
    /// rescan.
    pub(crate) fn render_keep_segmented(&self, cx: &mut Context<Self>) -> impl IntoElement {
        const MODES: [&str; 5] = ["First", "Smallest", "Newest", "Oldest", "Best quality"];
        let active = self.keep_index.unwrap_or(0).min(4);
        let off = self.scanning || self.groups.is_empty();
        let mut group = ButtonGroup::new("keep-mode").compact().disabled(off);
        for (i, label) in MODES.iter().enumerate() {
            group = group.child(
                Button::new(SharedString::from(format!("keep-mode-{i}")))
                    .label(*label)
                    .selected(i == active),
            );
        }
        group.on_click(cx.listener(move |this, clicks: &Vec<usize>, _, cx| {
            if let Some(&i) = clicks.first() {
                this.apply_keep_mode(i, cx);
            }
        }))
    }
    /// Toggle: default discovery order vs biggest-reclaimable-first.
    pub(crate) fn render_sort_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = usize::from(self.sort_biggest);
        let off = self.scanning || self.groups.is_empty();
        ButtonGroup::new("sort-mode")
            .compact()
            .disabled(off)
            .child(
                Button::new("sort-default")
                    .label("Default")
                    .selected(active == 0),
            )
            .child(
                Button::new("sort-biggest")
                    .label("Biggest")
                    .selected(active == 1),
            )
            .on_click(cx.listener(move |this, clicks: &Vec<usize>, _, cx| {
                if let Some(&i) = clicks.first() {
                    this.sort_biggest = i == 1;
                    // The toggle is THE explicit order change: sort now.
                    this.sort_groups();
                    this.save_prefs(cx);
                    cx.notify();
                }
            }))
    }
    pub(crate) fn render_delete_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let targets = self.pending_targets();
        let pending_bytes: u64 = targets.iter().map(|t| t.size).sum();
        let can_undo = !self.scanning && !self.last_trashed.is_empty();
        let can_scan_actions = !self.scanning && !self.groups.is_empty();
        let can_export = !self.scanning && self.has_scanned;
        let undo_count = self.last_trashed.len();
        let consolidate_label = match &self.consolidate_dest {
            Some(dest) => format!(
                "Move keepers to {}",
                dest.file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| dest.display().to_string())
            ),
            None => "Consolidate…".to_string(),
        };
        let view = cx.entity();
        let menu_view = view.clone();
        let delete_label: SharedString = if targets.is_empty() {
            "Delete selected".into()
        } else {
            format!("Delete {}", targets.len()).into()
        };
        let delete_tooltip: SharedString = if targets.is_empty() {
            "Nothing selected yet — tick the copies you want gone".into()
        } else {
            format!(
                "Delete {} selected file(s) ({}) — keepers are never touched",
                targets.len(),
                util::human_bytes(pending_bytes)
            )
            .into()
        };
        DropdownButton::new("delete-actions")
            .button(
                Button::new("delete")
                    .icon(IconName::Delete)
                    .label(delete_label)
                    .tooltip(delete_tooltip)
                    .disabled(self.scanning || self.groups.is_empty() || targets.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_delete_confirm(window, cx);
                    })),
            )
            .menu_accessibility_label("More run actions")
            .dropdown_menu(move |menu, _, _| {
                let save_view = menu_view.clone();
                let undo_view = menu_view.clone();
                let consolidate_view = menu_view.clone();
                menu.item(
                    PopupMenuItem::new("Save JSON…")
                        .icon(IconName::FileText)
                        .disabled(!can_export)
                        .on_click(move |_, window, cx| {
                            save_view.update(cx, |this, cx| this.save_report(window, cx));
                        }),
                )
                .item(
                    PopupMenuItem::new(format!("Undo delete ({undo_count})"))
                        .icon(IconName::Undo2)
                        .disabled(!can_undo)
                        .on_click(move |_, _, cx| {
                            undo_view.update(cx, |this, cx| this.undo_delete(cx));
                        }),
                )
                .item(
                    PopupMenuItem::new(consolidate_label.clone())
                        .icon(IconName::FolderOpen)
                        .disabled(!can_scan_actions)
                        .on_click(move |_, _, cx| {
                            consolidate_view.update(cx, |this, cx| {
                                if this.consolidate_dest.is_some() {
                                    this.perform_consolidate(cx);
                                } else {
                                    this.pick_consolidate_dir(cx);
                                }
                            });
                        }),
                )
            })
            .into_any_element()
    }
}
