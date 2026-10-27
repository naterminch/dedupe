use crate::ui::DedupeView;
use gpui_kit::{AppContext as _, Size, TitlebarOptions, WindowOptions, px};

/// Boot the GUI application. Returns when the window is closed.
pub fn run() {
    // Instant is Copy; both framework callbacks are 'static, so each gets
    // its own copy of the start time.
    let t_start = std::time::Instant::now();
    gpui_kit::application()
        .with_assets(crate::assets::AppAssets)
        .run(move |cx| {
            eprintln!("[startup] application ready in {:?}", t_start.elapsed());
            gpui_kit::init(cx);
            eprintln!("[startup] kit init done in {:?}", t_start.elapsed());
            gpui_kit::open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("dedupe".into()),
                        ..Default::default()
                    }),
                    window_min_size: Some(Size {
                        width: px(1120.),
                        height: px(640.),
                    }),
                    ..WindowOptions::default()
                },
                cx,
                move |window, cx| {
                    let view = cx.new(|cx| DedupeView::new(window, cx));
                    eprintln!("[startup] window + view built in {:?}", t_start.elapsed());
                    view
                },
            )
            .expect("failed to open dedupe window");
        });
}
