use crate::ui::{DedupeView, GROUP_PAGE};
use gpui_kit::Context;
use std::collections::HashMap;
use std::path::PathBuf;

impl DedupeView {
    /// Mark every non-keep member for deletion (fresh intent after a
    /// keeper/mode change).
    pub(crate) fn reselect_all_dups(&mut self) {
        self.selected = self.all_dup_paths();
    }
    /// Promote one member to keeper of its group (per-group decision).
    /// Member order never changes here: the grid keeps every card in place
    /// and only the highlight (KEEP/DUP badge + border) moves.
    pub(crate) fn set_keeper(
        &mut self,
        group_index: usize,
        path: &std::path::Path,
        cx: &mut Context<Self>,
    ) {
        if let Some(g) = self.groups.iter_mut().find(|g| g.index == group_index) {
            for m in g.members.iter_mut() {
                m.keep = m.path == path;
            }
        }
        self.reselect_all_dups();
        self.sync_keep_to_base();
        // Keeper swap changes which bytes count as reclaimable.
        self.recount();
        // Keeper change: disarm a pending consolidate (stale paths).
        self.consolidate_dest = None;
        cx.notify();
    }
    /// Apply a new global keep mode to the current results (no rescan).
    pub(crate) fn apply_keep_mode(&mut self, index: usize, cx: &mut Context<Self>) {
        self.keep_index = Some(index);
        let keep = match index {
            1 => crate::matching::KeepMode::Smallest,
            2 => crate::matching::KeepMode::Newest,
            3 => crate::matching::KeepMode::Oldest,
            4 => crate::matching::KeepMode::BestQuality,
            _ => crate::matching::KeepMode::First,
        };
        crate::matching::reassign_keepers(&mut self.groups, keep);
        self.reselect_all_dups();
        // New keepers everywhere: disarm a pending consolidate (stale paths).
        self.consolidate_dest = None;
        let name = [
            "First found",
            "Smallest",
            "Newest",
            "Oldest",
            "Best quality",
        ][index.min(4)];
        self.status = format!(
            "Keep mode: {name} applied to {} group(s).",
            self.groups.len()
        );
        self.sync_keep_to_base();
        // New keepers everywhere: re-apply the display order once, here.
        self.sort_groups();
        self.save_prefs(cx);
        cx.notify();
    }
    /// Recompute the summary counters from the current groups.
    pub(crate) fn recount(&mut self) {
        let dups: u64 = self
            .groups
            .iter()
            .map(|g| (g.members.len() - g.keep_count()) as u64)
            .sum();
        self.dup_files = dups;
        self.reclaim_bytes = self.groups.iter().map(|g| g.dup_bytes()).sum();
    }
    /// Order `groups` for display: biggest-reclaimable-first while the sort
    /// toggle is on, otherwise the scan's discovery order (via base_groups).
    /// Stable sort, so ties never jitter.
    ///
    /// Called ONLY on explicit order-changing events (new scan, sort toggle,
    /// similarity filter, global keep mode) — never on keeper picks or
    /// renders, so clicking a card can't reshuffle the list underneath.
    pub(crate) fn sort_groups(&mut self) {
        if self.sort_biggest {
            self.groups
                .sort_by_key(|b| std::cmp::Reverse(b.dup_bytes()));
        } else {
            let pos: HashMap<usize, usize> = self
                .base_groups
                .iter()
                .enumerate()
                .map(|(i, g)| (g.index, i))
                .collect();
            self.groups
                .sort_by_key(|g| pos.get(&g.index).copied().unwrap_or(usize::MAX));
        }
    }
    /// Mirror keep flags and member order from the live view back into the
    /// pristine scan results, so re-thresholding never loses keeper choices.
    /// Members hidden by the floor keep their stored flags.
    pub(crate) fn sync_keep_to_base(&mut self) {
        for g in &self.groups {
            if let Some(base) = self.base_groups.iter_mut().find(|b| b.index == g.index) {
                for m in &g.members {
                    if let Some(bm) = base.members.iter_mut().find(|bm| bm.path == m.path) {
                        bm.keep = m.keep;
                    }
                }
                let order: HashMap<PathBuf, usize> = g
                    .members
                    .iter()
                    .enumerate()
                    .map(|(i, m)| (m.path.clone(), i))
                    .collect();
                base.members
                    .sort_by_key(|m| order.get(&m.path).copied().unwrap_or(usize::MAX));
            }
        }
    }
    /// Re-derive the live groups from the pristine scan results at floor
    /// `t` (fraction). Exact groups always qualify; similar members need
    /// similarity-to-keeper >= `t`. Groups left with fewer than 2 members
    /// drop out (restorable by lowering the floor — no rescan).
    pub(crate) fn apply_threshold(&mut self, t: f64, cx: &mut Context<Self>) {
        let t = t.clamp(self.scan_similarity, 1.0);
        self.min_sim = t;
        self.groups = self
            .base_groups
            .iter()
            .filter_map(|g| {
                let members: Vec<_> = g
                    .members
                    .iter()
                    .filter(|m| m.similarity.is_none_or(|s| s >= t))
                    .cloned()
                    .collect();
                if members.len() >= 2 {
                    let mut g = g.clone();
                    g.members = members;
                    Some(g)
                } else {
                    None
                }
            })
            .collect();
        self.group_limit = GROUP_PAGE;
        self.reselect_all_dups();
        self.recount();
        // Fresh member set: re-apply the display order once, here.
        self.sort_groups();
        self.status = format!(
            "Similarity ≥ {:.0}% — {} group(s), {} duplicate file(s).",
            t * 100.0,
            self.groups.len(),
            self.dup_files,
        );
        cx.notify();
    }
    /// Parse the live floor box: blank restores the scan floor, a number
    /// tightens it, anything below the scan floor is clamped (loosening
    /// needs discarded pairs — rescan instead).
    pub(crate) fn apply_sim_input(&mut self, raw: &str, cx: &mut Context<Self>) {
        let raw = raw.trim();
        if raw.is_empty() {
            if (self.min_sim - self.scan_similarity).abs() > f64::EPSILON {
                let floor = self.scan_similarity;
                self.apply_threshold(floor, cx);
            }
            return;
        }
        match raw.parse::<f64>() {
            Ok(v) if (0.0..=100.0).contains(&v) => {
                let t = v / 100.0;
                if t < self.scan_similarity {
                    self.status = format!(
                        "Floor {:.0}% is below the scan threshold {:.0}% — rescan to loosen.",
                        v,
                        self.scan_similarity * 100.0
                    );
                    cx.notify();
                    return;
                }
                self.apply_threshold(t, cx);
            }
            _ => {}
        }
    }
}
