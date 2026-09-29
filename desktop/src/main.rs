#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use gpui::{prelude::*, *};
use gpui_component::{Root, Theme, ThemeMode};
mod app;
mod events;
mod platform;
mod views;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--version") {
        println!("Lightmail {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let demo = args.iter().any(|s| s == "--demo");
    let argument = |key: &str| {
        args.iter()
            .position(|s| s == key)
            .and_then(|i| args.get(i + 1))
            .map(std::path::PathBuf::from)
    };
    let acceptance = argument("--acceptance-dir");
    if acceptance.is_some() && !demo {
        eprintln!("Acceptance reports require --demo");
        std::process::exit(2);
    }
    let root = argument("--data-dir").unwrap_or_else(|| {
        directories::ProjectDirs::from("com", "Bailey", "Lightmail")
            .expect("User data directory")
            .data_local_dir()
            .to_path_buf()
    });
    let engine = lightmail_core::MailEngine::new(
        root.join(if demo { "Preview" } else { "Mail" })
            .to_string_lossy()
            .into_owned(),
    )
    .expect("Open local mail database");
    if demo {
        engine.seed_demo().expect("Prepare synthetic demo");
    }
    Application::new().run(move |cx| {
        gpui_component::init(cx);
        Theme::change(ThemeMode::Light, None, cx);
        Theme::global_mut(cx).colors.primary = rgb(0x226451).into();
        #[cfg(windows)]
        {
            Theme::global_mut(cx).font_family = "Microsoft YaHei UI".into();
        }
        let bounds = Bounds::centered(None, size(px(1280.), px(800.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(1040.), px(640.))),
                ..Default::default()
            },
            |window, cx| {
                window.set_window_title("轻邮 Lightmail");
                let view = cx.new(|cx| app::MailDesktop::new(engine, demo, acceptance, window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("Open Lightmail window");
        cx.activate(true);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    });
}
