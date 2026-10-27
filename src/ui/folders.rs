use crate::ui::{DedupeView, FolderEntry};
use gpui_kit::{Context, SharedString, Window};

impl DedupeView {
    /// Add folders to the scan list (deduplicated, in order).
    pub(crate) fn add_folders(&mut self, raw: &str, cx: &mut Context<Self>) {
        for p in raw
            .split([';', '\n'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            if !self.folders.iter().any(|f| f.path == p) {
                self.folders.push(FolderEntry {
                    path: p.to_string(),
                    reference: false,
                });
            }
        }
        cx.notify();
        self.save_prefs(cx);
    }
    pub(crate) fn add_from_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let raw = self.folder_input.read(cx).value().trim().to_string();
        self.add_folders(&raw, cx);
        self.folder_input.update(cx, |input, cx| {
            input.set_value(SharedString::from(String::new()), window, cx);
        });
    }
    pub(crate) fn browse(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let start = self
            .folders
            .last()
            .map(|f| f.path.clone())
            .unwrap_or_default();
        // Async dialog on rfd's own thread: a blocking native dialog must
        // never run inside a GPUI event handler — its modal message loop
        // re-enters GPUI dispatch and aborts on the borrowed app context.
        cx.spawn(async move |this, _cx| {
            let mut dlg = rfd::AsyncFileDialog::new().set_title("Choose a folder to scan");
            if !start.is_empty() && std::path::Path::new(&start).exists() {
                dlg = dlg.set_directory(&start);
            }
            let picked = dlg.pick_folder().await;
            if let Some(handle) = picked {
                let path = handle.path().display().to_string();
                this.update(_cx, |view, cx| {
                    view.add_folders(&path, cx);
                })
                .ok();
            }
        })
        .detach();
    }
}
