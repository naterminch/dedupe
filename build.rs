fn main() {
    // Windows .exe icon (taskbar, Explorer, Alt-Tab): no-op elsewhere.
    #[cfg(windows)]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        if let Err(e) = res.compile() {
            eprintln!("warning: could not embed exe icon: {e}");
        }
    }
}
