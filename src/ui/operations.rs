use crate::ui::{DedupeView, ThemeReadability};
use crate::ui_helpers::fit_text;
use crate::{actions, hashing, util};
use gpui_kit::base::{h_flex, v_flex};
use gpui_kit::component::button::{Button, ButtonVariant};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, WindowExt};
use gpui_kit::{
    Context, FontWeight, ParentElement as _, SharedString, Styled as _, Window, div, px,
};
use std::collections::HashSet;
use std::path::PathBuf;

impl DedupeView {
    /// Files currently marked for deletion (selected checkboxes that are
    /// still non-keep members of a group).
    pub(crate) fn pending_targets(&self) -> Vec<actions::DeleteTarget> {
        let mut out = Vec::new();
        for g in &self.groups {
            for m in &g.members {
                if !m.keep && self.selected.contains(&m.path) {
                    out.push(actions::DeleteTarget {
                        path: m.path.clone(),
                        expected_hash: m.content_hash.clone().unwrap_or_else(|| g.hash.clone()),
                        size: m.size,
                    });
                }
            }
        }
        out
    }
    pub(crate) fn perform_delete(&mut self, cx: &mut Context<Self>) {
        let targets = self.pending_targets();
        if targets.is_empty() {
            self.status = "Tick the duplicate copies you want deleted first.".to_string();
            cx.notify();
            return;
        }
        let engine = hashing::HashEngine::new(self.last_hash);
        let disposition = if self.trash {
            actions::Disposition::Trash
        } else {
            actions::Disposition::Permanent
        };
        let outcome = actions::delete_targets(&targets, &engine, disposition);
        let gone: HashSet<PathBuf> = outcome.deleted.iter().cloned().collect();
        for g in &mut self.groups {
            g.members.retain(|m| !gone.contains(&m.path));
        }
        self.groups.retain(|g| g.members.len() >= 2);
        // Prune the pristine copy too so a later floor change cannot
        // resurrect deleted files.
        for g in &mut self.base_groups {
            g.members.retain(|m| !gone.contains(&m.path));
        }
        self.base_groups.retain(|g| g.members.len() >= 2);
        for p in &outcome.deleted {
            self.selected.remove(p);
        }
        // Deleted paths are gone: disarm a pending consolidate.
        self.consolidate_dest = None;
        let verb = if self.trash {
            "Moved to trash"
        } else {
            "Deleted"
        };
        // Trash receipts enable one-step undo; permanent deletes cannot
        // be undone, so any stale receipt is dropped.
        if self.trash {
            self.last_trashed = outcome.deleted.clone();
        } else {
            self.last_trashed.clear();
        }
        self.status = format!(
            "{verb} {} file(s) ({} freed); {} skipped.",
            outcome.deleted.len(),
            util::human_bytes(outcome.bytes_freed),
            outcome.skipped.len()
        );
        cx.notify();
    }
    /// First click of the two-click consolidate: pick the destination
    /// folder, then preview the planned keeper moves in the status line.
    /// The second click (same button) executes via `perform_consolidate`.
    pub(crate) fn pick_consolidate_dir(&mut self, cx: &mut Context<Self>) {
        let start = self
            .folders
            .last()
            .map(|f| f.path.clone())
            .unwrap_or_default();
        // Same rule as browse(): native dialogs stay off the GPUI thread.
        cx.spawn(async move |this, _cx| {
            let mut dlg =
                rfd::AsyncFileDialog::new().set_title("Choose the consolidate folder");
            if !start.is_empty() && std::path::Path::new(&start).exists() {
                dlg = dlg.set_directory(&start);
            }
            let picked = dlg.pick_folder().await;
            let Some(handle) = picked else { return };
            let dest = handle.path().to_path_buf();
            this.update(_cx, |view, cx| {
                let (moves, skipped) = actions::plan_consolidation(&view.groups, &dest);
                if moves.is_empty() {
                    view.consolidate_dest = None;
                    view.status =
                        "Nothing to move — every keeper is protected or already in that folder."
                            .to_string();
                } else {
                    let bytes: u64 = moves.iter().map(|m| m.target.size).sum();
                    view.consolidate_dest = Some(dest.clone());
                    view.status = format!(
                        "Consolidate {} keeper(s) ({}) into {}? ({} skipped) Press “Move keepers” again to move them.",
                        moves.len(),
                        util::human_bytes(bytes),
                        dest.display(),
                        skipped.len(),
                    );
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
    /// Second click: move every group's keeper into the armed destination.
    /// The plan is rebuilt live so retries after partial moves stay
    /// correct; keepers are re-hashed before moving and member paths plus
    /// ticks follow moved files.
    pub(crate) fn perform_consolidate(&mut self, cx: &mut Context<Self>) {
        let Some(dest) = self.consolidate_dest.clone() else {
            self.status = "Choose the folder that keepers should be moved into.".to_string();
            cx.notify();
            return;
        };
        let (moves, skipped) = actions::plan_consolidation(&self.groups, &dest);
        if moves.is_empty() {
            self.status = "Nothing to move — every keeper is protected or already in that folder."
                .to_string();
            self.consolidate_dest = None;
            cx.notify();
            return;
        }
        let engine = hashing::HashEngine::new(self.last_hash);
        let targets: Vec<actions::MoveTarget> = moves.iter().map(|m| m.target.clone()).collect();
        let outcome = actions::consolidate_targets(&targets, &engine);
        for (from, to) in &outcome.moved {
            for groups in [&mut self.groups, &mut self.base_groups] {
                for m in groups.iter_mut().flat_map(|g| g.members.iter_mut()) {
                    if m.path == *from {
                        m.path = to.clone();
                    }
                }
            }
            // Ticks follow moved files.
            if self.selected.remove(from) {
                self.selected.insert(to.clone());
            }
        }
        self.consolidate_dest = None;
        self.status = format!(
            "Consolidated {} keeper(s) ({}) into {}; {} skipped.",
            outcome.moved.len(),
            util::human_bytes(outcome.bytes_moved),
            dest.display(),
            outcome.skipped.len() + skipped.len(),
        );
        cx.notify();
    }
    /// Restore the last trash run (undo). Permanent deletes have no
    /// receipt and cannot be restored. Restored files reappear on disk;
    /// press Scan to refresh the groups.
    pub(crate) fn undo_delete(&mut self, cx: &mut Context<Self>) {
        if self.last_trashed.is_empty() {
            self.status = "Nothing to restore — nothing has been deleted yet.".to_string();
            cx.notify();
            return;
        }
        // `trash::os_limited` (list/restore) only exists on Windows and
        // Freedesktop-trash Unix — there is no public macOS API for it, so
        // the module is cfg-gated out there. Mirror the crate's gate here.
        #[cfg(any(
            target_os = "windows",
            all(
                unix,
                not(target_os = "macos"),
                not(target_os = "ios"),
                not(target_os = "android")
            )
        ))]
        {
            let wanted: HashSet<PathBuf> = self.last_trashed.iter().cloned().collect();
            let items = match trash::os_limited::list() {
                Ok(items) => items,
                Err(e) => {
                    self.status = format!("Could not read the trash: {e}");
                    cx.notify();
                    return;
                }
            };
            let mine: Vec<_> = items
                .into_iter()
                .filter(|it| wanted.contains(&it.original_path()))
                .collect();
            if mine.is_empty() {
                self.status = "Nothing to restore — the trash was already emptied.".to_string();
                self.last_trashed.clear();
                cx.notify();
                return;
            }
            let n = mine.len();
            match trash::os_limited::restore_all(mine) {
                Ok(()) => {
                    self.status = format!("Restored {n} file(s). Press Scan to refresh.");
                    self.last_trashed.clear();
                }
                Err(e) => {
                    self.status = format!("Restore failed: {e}");
                }
            }
            cx.notify();
        }
        #[cfg(not(any(
            target_os = "windows",
            all(
                unix,
                not(target_os = "macos"),
                not(target_os = "ios"),
                not(target_os = "android")
            )
        )))]
        {
            self.last_trashed.clear();
            self.status =
                "Undo is not supported on this platform (trash restore needs Windows/Linux)."
                    .to_string();
            cx.notify();
        }
    }
    /// Delete confirmation as a modal dialog: a per-group dry-run summary
    /// (keeper + exactly what will go, with sizes) plus in-dialog Keep
    /// picks. After a Keep change the dialog reopens fresh so it never
    /// shows stale keep/dup state.
    pub(crate) fn open_delete_confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let targets = self.pending_targets();
        if targets.is_empty() {
            self.status = "Tick the duplicate copies you want deleted first.".to_string();
            cx.notify();
            return;
        }
        let count = targets.len();
        let bytes: u64 = targets.iter().map(|t| t.size).sum();
        let bytes_str = util::human_bytes(bytes);
        let action = if self.trash {
            "move to the trash"
        } else {
            "permanently delete"
        };
        let view = cx.entity();
        // Per-group dry-run data, capped so huge selections stay readable.
        // The dialog closure must stay `Fn`, so everything is owned here.
        const CONFIRM_GROUPS: usize = 6;
        const CONFIRM_ROWS: usize = 4;
        struct ConfirmGroup {
            index: usize,
            keeper: String,
            rows: Vec<(String, u64, PathBuf)>,
            hidden_rows: usize,
            group_bytes: u64,
        }
        let mut confirm_groups: Vec<ConfirmGroup> = Vec::new();
        for g in &self.groups {
            let mut rows = Vec::new();
            let mut group_bytes = 0u64;
            let mut total = 0usize;
            for m in &g.members {
                if !m.keep && self.selected.contains(&m.path) {
                    total += 1;
                    group_bytes += m.size;
                    if rows.len() < CONFIRM_ROWS {
                        rows.push((m.path.display().to_string(), m.size, m.path.clone()));
                    }
                }
            }
            if total == 0 {
                continue;
            }
            let keeper = g
                .members
                .iter()
                .find(|m| m.keep)
                .map(|m| {
                    m.path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| m.path.display().to_string())
                })
                .unwrap_or_else(|| "—".to_string());
            confirm_groups.push(ConfirmGroup {
                index: g.index,
                keeper,
                rows,
                hidden_rows: total.saturating_sub(CONFIRM_ROWS),
                group_bytes,
            });
            if confirm_groups.len() >= CONFIRM_GROUPS {
                break;
            }
        }
        let hidden_groups = self
            .groups
            .iter()
            .filter(|g| {
                g.members
                    .iter()
                    .any(|m| !m.keep && self.selected.contains(&m.path))
            })
            .count()
            .saturating_sub(confirm_groups.len());
        let dialog_xs = px(cx.theme().text_xs_px());
        window.open_alert_dialog(
            cx,
            move |alert, _, _| {
                let confirm_view = view.clone();
                let mut body = v_flex().gap_2();
                for cg in &confirm_groups {
                    let mut section = v_flex()
                        .gap_1()
                        .py_1()
                        .child(
                            div()
                                .text_size(dialog_xs)
                                .font_weight(FontWeight::BOLD)
                                .child(format!(
                                    "Group #{} · keeps {} · deletes {} ({})",
                                    cg.index,
                                    cg.keeper,
                                    cg.rows.len() + cg.hidden_rows,
                                    util::human_bytes(cg.group_bytes),
                                )),
                        );
                    for (name, size, path) in &cg.rows {
                        let gi = cg.index;
                        let keep_path = path.clone();
                        let reopen_view = confirm_view.clone();
                        section = section.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .text_size(dialog_xs)
                                        .truncate()
                                        .child(format!(
                                            "{} ({})",
                                            fit_text(name, 70),
                                            util::human_bytes(*size)
                                        )),
                                )
                                .child(
                                    Button::new(SharedString::from(format!(
                                        "dlg-keep-{gi}-{}",
                                        fit_text(name, 20)
                                    )))
                                    .label("Keep")
                                    .on_click(move |_, window, cx| {
                                        reopen_view.update(cx, |v, cx| {
                                            v.set_keeper(gi, &keep_path, cx);
                                        });
                                        window.close_dialog(cx);
                                        let reopen_view2 = reopen_view.clone();
                                        reopen_view2.update(cx, |v, cx| {
                                            v.open_delete_confirm(window, cx);
                                        });
                                    }),
                                ),
                        );
                    }
                    if cg.hidden_rows > 0 {
                        section = section.child(
                            div()
                                .text_size(dialog_xs)
                                .child(format!("… and {} more in this group", cg.hidden_rows)),
                        );
                    }
                    body = body.child(section);
                }
                if hidden_groups > 0 {
                    body = body.child(
                        div()
                            .text_size(dialog_xs)
                            .child(format!("… and {hidden_groups} more group(s)")),
                    );
                }
                alert
                    .title(format!("Delete {count} file(s)?"))
                    .description(format!(
                        "This will {action} {count} file(s) ({bytes_str}). Keepers are kept; every file is re-hashed before removal."
                    ))
                    .child(body)
                    .confirm()
                    .ok_text(format!("Delete {count}"))
                    .ok_variant(ButtonVariant::Danger)
                    .on_ok(move |_, window, cx| {
                        let message = confirm_view.update(cx, |v, cx| {
                            v.perform_delete(cx);
                            v.status.clone()
                        });
                        window.push_notification(
                            Notification::success(message).autohide(true),
                            cx,
                        );
                        true
                    })
            },
        );
    }
}
