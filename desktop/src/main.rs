#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use gpui::{prelude::*, *};
use gpui_component::{Root, Theme, ThemeMode};
#[cfg(feature = "acceptance")]
mod acceptance;
mod app;
mod assets;
mod events;
mod platform;
mod shortcuts;
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
    #[cfg(feature = "acceptance")]
    let run_acceptance = args.iter().any(|s| s == "--run-acceptance");
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
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui_component::init(cx);
            shortcuts::bind(cx);
            Theme::change(ThemeMode::Light, None, cx);
            Theme::global_mut(cx).colors.primary = rgb(0x226451).into();
            Theme::global_mut(cx).font_size = px(13.);
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
                    #[cfg(feature = "acceptance")]
                    let acceptance_path = acceptance.clone();
                    let view =
                        cx.new(|cx| app::MailDesktop::new(engine, demo, acceptance, window, cx));
                    #[cfg(feature = "acceptance")]
                    if run_acceptance && demo {
                        if let Some(path) = acceptance_path {
                            acceptance::start(view.clone(), window, path, cx);
                        }
                    }
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
