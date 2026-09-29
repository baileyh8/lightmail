#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{prelude::*, *};
#[cfg(feature = "acceptance")]
mod acceptance;
mod app;
mod assets;
mod events;
mod images;
mod instance;
mod platform;
mod reader;
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
    if demo {
        if let Some(path) = acceptance.clone() {
            let _ = std::fs::create_dir_all(&path);
            std::panic::set_hook(Box::new(move |info| {
                let trace = std::backtrace::Backtrace::force_capture();
                let _ = std::fs::write(path.join("panic.txt"), format!("{info}\n{trace}"));
            }));
        }
    }
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
    let directory = root.join(if demo { "Preview" } else { "Mail" });
    // Held until the process exits, beyond Application::run.
    let _instance = match instance::DirectoryLock::acquire_within(
        &directory,
        std::time::Duration::from_secs(3),
    ) {
        Ok(lock) => lock,
        Err(error) => {
            let running = error.kind() == std::io::ErrorKind::WouldBlock;
            rfd::MessageDialog::new()
                .set_level(rfd::MessageLevel::Info)
                .set_title("轻邮 Lightmail")
                .set_description(if running {
                    "轻邮已经在运行。同一个邮箱数据目录只能由一个窗口打开。".to_string()
                } else {
                    format!("无法打开邮箱数据目录：{error}")
                })
                .set_buttons(rfd::MessageButtons::Ok)
                .show();
            std::process::exit(if running { 0 } else { 1 });
        }
    };
    let engine = lightmail_core::MailEngine::new(directory.to_string_lossy().into_owned())
        .expect("Open local mail database");
    if demo {
        engine.seed_demo().expect("Prepare synthetic demo");
    }
    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            shortcuts::bind(cx);
            Theme::change(ThemeMode::Light, None, cx);
            // Kit's update path keeps component and base theme tokens in sync.
            Theme::update(cx, |theme| {
                let accent: Hsla = rgb(0x226451).into();
                theme.colors.primary = accent;
                theme.colors.button_primary = accent;
                theme.colors.ring = accent;
                theme.font_size = px(13.);
                #[cfg(windows)]
                {
                    theme.font_family = "Microsoft YaHei UI".into();
                }
            });
            // Keep the complete window visible on smaller Windows desktops and at
            // larger display scaling factors. Dimensions here are logical pixels.
            let screen = cx
                .primary_display()
                .map(|display| display.bounds().size)
                .unwrap_or(size(px(1280.), px(800.)));
            let initial_size = size(
                px(1280.).min(screen.width - px(32.)),
                px(800.).min(screen.height - px(80.)),
            );
            let bounds = Bounds::centered(None, initial_size, cx);
            #[cfg(windows)]
            let bounds = platform::initial_window_bounds().unwrap_or(bounds);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(
                        px(1040.).min(bounds.size.width),
                        px(640.).min(bounds.size.height),
                    )),
                    ..Default::default()
                },
                cx,
                |window, cx| {
                    window.set_window_title("轻邮 Lightmail");
                    #[cfg(feature = "acceptance")]
                    let acceptance_path = acceptance.clone();
                    let view = cx.new(|cx| {
                        app::MailDesktop::new(engine, root, demo, acceptance, window, cx)
                    });
                    #[cfg(feature = "acceptance")]
                    if run_acceptance && demo {
                        if let Some(path) = acceptance_path {
                            acceptance::start(view.clone(), window, path, cx);
                        }
                    }
                    view
                },
            )
            .expect("Open Lightmail window");
            cx.activate(true);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
        });
}
