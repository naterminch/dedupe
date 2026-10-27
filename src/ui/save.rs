use crate::report;
use crate::ui::{DedupeView, ReportSnapshot};
use gpui_kit::{Context, Window};

impl DedupeView {
    /// Write the current results as JSON (same schema as `--json`).
    pub(crate) fn save_report(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(stats) = self.last_stats.clone() else {
            self.status = "Nothing to save yet — run a scan first.".to_string();
            cx.notify();
            return;
        };
        // Snapshot everything the async dialog task needs; the view may
        // change (or close) while the native dialog is open.
        let snapshot = ReportSnapshot {
            paths: self.last_paths.clone(),
            hash_algo: self.last_hash_algo.clone(),
            ffprobe_used: self.last_ffprobe,
            stats,
            groups: self.groups.clone(),
        };
        // Same rule as browse(): native dialogs stay off the GPUI thread.
        cx.spawn(async move |this, cx| {
            let file = rfd::AsyncFileDialog::new()
                .set_title("Save duplicate report")
                .set_file_name("dedupe-report.json")
                .save_file()
                .await;
            let Some(file) = file else { return };
            let ctx = report::ReportContext {
                paths: &snapshot.paths,
                hash_algo: &snapshot.hash_algo,
                verbose: false,
                ffprobe_used: snapshot.ffprobe_used,
            };
            let json = report::to_json(&ctx, &snapshot.stats, &snapshot.groups);
            let outcome = std::fs::write(file.path(), json);
            this.update(cx, |view, cx| {
                match outcome {
                    Ok(()) => {
                        view.status = format!("Saved report to {}.", file.path().display());
                        view.pending_toast = Some((view.status.clone(), false));
                    }
                    Err(e) => {
                        view.status = format!("Could not save report: {e}");
                        view.pending_toast = Some((view.status.clone(), true));
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
