//! Opt-in native UI acceptance runner. Only compiled for CI; only uses demo data.
use crate::{
    app::{MailDesktop, Page},
    platform::DesktopPlatform,
    reader::Document,
};
use gpui_kit::*;
use lightmail_core::{ExportMode, PlatformServices};
use std::{path::PathBuf, sync::Arc, time::Duration};

async fn pause(cx: &AsyncApp) {
    cx.background_executor()
        .timer(Duration::from_millis(250))
        .await;
}
async fn screenshot(path: &std::path::Path, name: &str, cx: &AsyncApp) -> anyhow::Result<()> {
    std::fs::create_dir_all(path)?;
    std::fs::write(path.join("screenshot-request.txt"), name)?;
    for _ in 0..24 {
        pause(cx).await;
        if path.join(format!("{name}.captured")).exists() {
            return Ok(());
        }
    }
    anyhow::bail!("Screenshot collector did not capture {name}")
}
#[cfg(windows)]
mod win32 {
    use gpui_kit::Window;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    pub use std::ffi::c_void;

    #[repr(C)]
    pub struct DevicePoint {
        pub x: i32,
        pub y: i32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    pub struct DeviceRect {
        pub left: i32,
        pub top: i32,
        pub right: i32,
        pub bottom: i32,
    }
    #[link(name = "user32")]
    extern "system" {
        pub fn PostMessageW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> i32;
        pub fn SendMessageW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> isize;
        pub fn ClientToScreen(hwnd: *mut c_void, point: *mut DevicePoint) -> i32;
        pub fn GetWindowRect(hwnd: *mut c_void, rect: *mut DeviceRect) -> i32;
    }
    pub fn hwnd(window: &Window) -> anyhow::Result<*mut c_void> {
        match HasWindowHandle::window_handle(window)
            .map_err(|e| anyhow::anyhow!("Window handle unavailable: {e:?}"))?
            .as_raw()
        {
            RawWindowHandle::Win32(raw) => Ok(raw.hwnd.get() as *mut c_void),
            _ => anyhow::bail!("Not a Windows window"),
        }
    }
    /// Packs device coordinates the way mouse messages carry them.
    pub fn coordinates(x: i32, y: i32) -> isize {
        ((y as i16 as u16 as u32) << 16 | x as i16 as u16 as u32) as isize
    }
}
fn bounds(
    view: &Entity<MailDesktop>,
    handle: AnyWindowHandle,
    name: &str,
    cx: &mut AsyncApp,
) -> anyhow::Result<Bounds<Pixels>> {
    handle.update(cx, |_, _, cx| {
        view.read(cx)
            .probes
            .get(name)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("Missing UI target {name}"))
    })?
}
fn click(
    view: &Entity<MailDesktop>,
    handle: AnyWindowHandle,
    name: &str,
    edge: bool,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let name = name.to_string();
    handle.update(cx, |_, window, cx| {
        let bounds = *view
            .read(cx)
            .probes
            .get(&name)
            .ok_or_else(|| anyhow::anyhow!("Missing UI target {name}"))?;
        anyhow::ensure!(
            bounds.size.width > px(20.) && bounds.size.height > px(15.),
            "Collapsed target {name}"
        );
        let position = if edge {
            point(bounds.right() - px(8.), bounds.bottom() - px(8.))
        } else {
            bounds.center()
        };
        if let Some(path) = &view.read(cx).acceptance {
            let trace = serde_json::json!({"target": name, "bounds": {"x": f32::from(bounds.left()), "y": f32::from(bounds.top()), "width": f32::from(bounds.size.width), "height": f32::from(bounds.size.height)}, "click": {"x": f32::from(position.x), "y": f32::from(position.y)}, "scale": window.scale_factor()});
            let _ = std::fs::create_dir_all(path);
            let _ = std::fs::write(path.join("last-click.json"), trace.to_string());
        }
        #[cfg(windows)]
        {
            let hwnd = win32::hwnd(window)?;
            let coordinates = win32::coordinates(
                (f32::from(position.x) * window.scale_factor()) as i32,
                (f32::from(position.y) * window.scale_factor()) as i32,
            );
            // Target this synthetic CI window only; never move the global desktop pointer.
            unsafe {
                for (message, buttons) in [(0x0200, 0), (0x0201, 1), (0x0202, 0)] {
                    anyhow::ensure!(
                        win32::PostMessageW(hwnd, message, buttons, coordinates) != 0,
                        "Native mouse message failed"
                    );
                }
            }
        }
        #[cfg(not(windows))]
        {
            let _ = (window, position);
            anyhow::bail!("Native mouse acceptance requires a Windows runner");
        }
        #[cfg(windows)]
        Ok(())
    })?
}
const HTCLIENT: isize = 1;
const HTCAPTION: isize = 2;
const HTMINBUTTON: isize = 8;
const HTMAXBUTTON: isize = 9;
const HTCLOSE: isize = 20;
/// The Win32 hit-test code at a logical point of the window. GPUI answers
/// WM_NCHITTEST from the last pointer position it saw, so a synthetic move to
/// the point comes first; the real desktop pointer never moves.
fn hit_test(
    handle: AnyWindowHandle,
    position: Point<Pixels>,
    cx: &mut AsyncApp,
) -> anyhow::Result<isize> {
    #[cfg(windows)]
    {
        let (hwnd, client, screen) = handle.update(cx, |_, window, _| {
            let hwnd = win32::hwnd(window)?;
            let mut point = win32::DevicePoint {
                x: (f32::from(position.x) * window.scale_factor()) as i32,
                y: (f32::from(position.y) * window.scale_factor()) as i32,
            };
            let client = win32::coordinates(point.x, point.y);
            anyhow::ensure!(
                unsafe { win32::ClientToScreen(hwnd, &mut point) } != 0,
                "Screen position unavailable"
            );
            Ok::<_, anyhow::Error>((hwnd as isize, client, win32::coordinates(point.x, point.y)))
        })??;
        // Sent back to back outside any update: the window procedure re-enters the
        // app to answer, and no real pointer input is read in between.
        let hwnd = hwnd as *mut win32::c_void;
        Ok(unsafe {
            win32::SendMessageW(hwnd, 0x0200, 0, client);
            win32::SendMessageW(hwnd, 0x0084, 0, screen)
        })
    }
    #[cfg(not(windows))]
    {
        let _ = (handle, position, cx);
        anyhow::bail!("Native hit testing requires a Windows runner")
    }
}
/// The drawn controls and the header drag areas answer with native codes,
/// buttons beside a drag area keep their clicks, and maximize also restores.
async fn window_chrome(
    view: &Entity<MailDesktop>,
    handle: AnyWindowHandle,
    path: &std::path::Path,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    for (name, expected) in [
        ("window-minimize", HTMINBUTTON),
        ("window-maximize", HTMAXBUTTON),
        ("window-close", HTCLOSE),
        ("brand-drag", HTCAPTION),
        ("list-title-drag", HTCAPTION),
        ("reader-toolbar-drag", HTCAPTION),
        ("compose", HTCLIENT),
        ("copy-markdown", HTCLIENT),
    ] {
        let position = bounds(view, handle, name, cx)?.center();
        let code = hit_test(handle, position, cx)?;
        anyhow::ensure!(
            code == expected,
            "{name} at {position:?} hit test returned {code}, expected {expected}"
        );
    }
    for maximized in [true, false] {
        click(view, handle, "window-maximize", false, cx)?;
        let mut settled = false;
        for _ in 0..20 {
            pause(cx).await;
            if handle.update(cx, |_, window, _| window.is_maximized())? == maximized {
                settled = true;
                break;
            }
        }
        anyhow::ensure!(
            settled,
            "Maximize control did not {}",
            if maximized { "maximize" } else { "restore" }
        );
        // Let the probes follow the new window size before the next click.
        for _ in 0..2 {
            pause(cx).await;
        }
        if maximized {
            screenshot(path, "windows-maximized", cx).await?;
        }
    }
    Ok(())
}
/// With a dialog open, its dimmed margin moves the window, the dialog keeps
/// its clicks, and the window controls stay above it.
async fn overlay_margin(
    view: &Entity<MailDesktop>,
    handle: AnyWindowHandle,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let panel = bounds(view, handle, "overlay-panel", cx)?;
    let margin = point(
        panel.center().x,
        panel.top() - px(crate::window_chrome::HEIGHT / 2.),
    );
    for (name, position, expected) in [
        ("dialog margin", margin, HTCAPTION),
        (
            "translation-tab",
            bounds(view, handle, "translation-tab", cx)?.center(),
            HTCLIENT,
        ),
        (
            "window-close",
            bounds(view, handle, "window-close", cx)?.center(),
            HTCLOSE,
        ),
    ] {
        let code = hit_test(handle, position, cx)?;
        anyhow::ensure!(
            code == expected,
            "{name} at {position:?} hit test returned {code}, expected {expected}"
        );
    }
    Ok(())
}
/// Opens the listed messages in turn, as a user would, and reports progress
/// so a collector can chart the working set per read; then idles for 30 s.
async fn soak(
    view: &Entity<MailDesktop>,
    handle: AnyWindowHandle,
    path: &std::path::Path,
    reads: usize,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    // Only rows shown in full: the list footer covers part of the last one.
    let rows: Vec<usize> = handle.update(cx, |_, _, cx| {
        let s = view.read(cx);
        let footer = s.probes.get("list-footer").map(|b| b.top());
        (0..s.messages.len())
            .filter(|i| {
                s.probes
                    .get(&format!("message-{i}"))
                    .is_some_and(|b| footer.is_none_or(|top| b.bottom() <= top))
            })
            .collect()
    })?;
    anyhow::ensure!(!rows.is_empty(), "No fully visible messages to read");
    for read in 0..reads {
        let index = rows[read % rows.len()];
        let id = handle.update(cx, |_, _, cx| view.read(cx).messages[index].id.clone())?;
        click(view, handle, &format!("message-{index}"), true, cx)?;
        let mut loaded = false;
        for _ in 0..40 {
            pause(cx).await;
            loaded = handle.update(cx, |_, _, cx| {
                let s = view.read(cx);
                s.selected.as_ref().is_some_and(|m| m.id == id) && s.body.is_some() && !s.loading
            })?;
            if loaded {
                break;
            }
        }
        anyhow::ensure!(loaded, "Read {read} did not open message {index}");
        let progress = serde_json::json!({ "read": read + 1, "of": reads });
        std::fs::write(path.join("soak-progress.json"), progress.to_string())?;
    }
    for _ in 0..120 {
        pause(cx).await;
    }
    Ok(())
}
// The native reader has no script engine or DOM: checks read the prepared
// document and the exact image and link policy the view renders with.
fn document_text(view: &Entity<MailDesktop>, cx: &App) -> Option<String> {
    Some(match view.read(cx).reader.as_ref()? {
        Document::Markdown(text) | Document::Html(text) => text.to_string(),
        Document::Blitz(rendered) => rendered.source.to_string(),
        Document::Bilingual(pairs) => pairs
            .iter()
            .map(|(source, target)| format!("{source} {target}"))
            .collect::<Vec<_>>()
            .join(" "),
    })
}
fn blocks_remote_images(images: &crate::reader::Images) -> bool {
    matches!(
        images(&SharedUri::from("https://example.invalid/pixel.png")),
        ImageSource::Resource(Resource::Embedded(path)) if path.as_ref() == crate::reader::PLACEHOLDER
    )
}

fn post_reader_pointer(
    handle: AnyWindowHandle,
    position: Point<Pixels>,
    message: u32,
    buttons: usize,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    handle.update(cx, |_, window, _| {
        #[cfg(windows)]
        {
            let packed = win32::coordinates(
                (f32::from(position.x) * window.scale_factor()) as i32,
                (f32::from(position.y) * window.scale_factor()) as i32,
            );
            anyhow::ensure!(
                unsafe { win32::PostMessageW(win32::hwnd(window)?, message, buttons, packed) } != 0,
                "Reader pointer message failed"
            );
            Ok(())
        }
        #[cfg(not(windows))]
        {
            let _ = (window, position, message, buttons);
            anyhow::bail!("Reader pointer acceptance needs Windows");
        }
    })?
}
async fn reader_selection(
    view: &Entity<MailDesktop>,
    handle: AnyWindowHandle,
    path: &std::path::Path,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let original = handle.update(cx, |_, window, _| {
        let original = window.viewport_size();
        window.resize(size(px(1120.), px(780.)));
        original
    })?;
    for _ in 0..40 {
        pause(cx).await;
        if handle.update(cx,|_,_,cx| {
            let s=view.read(cx);
            matches!(s.reader.as_ref(),Some(Document::Blitz(r)) if (r.viewport.width as f32-f32::from(s.probes["reader-viewport"].size.width)).abs()<=1. &&r.source.contains("Alpha 中文 Beta"))
        })? { break; }
    }
    handle.update(cx, |_, window, cx| {
        let s = view.read(cx);
        let Some(Document::Blitz(r)) = &s.reader else {
            anyhow::bail!("Reader absent after resize");
        };
        anyhow::ensure!(
            (r.viewport.width as f32 - f32::from(s.probes["reader-viewport"].size.width)).abs()
                <= 1.,
            "Reader ignored actual pane width"
        );
        let focus = s.reader_focus.clone();
        window.focus(&focus, cx);
        window.dispatch_action(Box::new(crate::shortcuts::SelectReaderAll), cx);
        Ok::<_, anyhow::Error>(())
    })??;
    for _ in 0..40 {
        pause(cx).await;
        if handle.update(cx, |_, _, cx| {
            view.read(cx)
                .reader_selection
                .text
                .contains("Alpha 中文 Beta")
        })? {
            break;
        }
    }
    let (start, end) = handle.update(cx, |_, _, cx| {
        let s = view.read(cx);
        anyhow::ensure!(
            s.reader_selection
                .text
                .contains("Scrollable synthetic paragraph 79"),
            "Select-all missed document tail"
        );
        let rect = s
            .reader_selection
            .rects
            .first()
            .ok_or_else(|| anyhow::anyhow!("Selection geometry absent"))?;
        let origin = s.reader_surface.origin;
        Ok::<_, anyhow::Error>((
            point(
                origin.x + px(rect.x + 1.),
                origin.y + px(rect.y + rect.height / 2.),
            ),
            point(
                origin.x + px(rect.x + rect.width - 1.),
                origin.y + px(rect.y + rect.height / 2.),
            ),
        ))
    })??;
    post_reader_pointer(handle, start, 0x0200, 0, cx)?;
    post_reader_pointer(handle, start, 0x0201, 1, cx)?;
    pause(cx).await;
    post_reader_pointer(handle, end, 0x0200, 1, cx)?;
    post_reader_pointer(handle, end, 0x0202, 0, cx)?;
    for _ in 0..40 {
        pause(cx).await;
        if handle.update(cx, |_, _, cx| {
            view.read(cx).reader_selection.text == "Alpha 中文 Beta"
        })? {
            break;
        }
    }
    handle.update(cx, |_, window, cx| {
        anyhow::ensure!(
            view.read(cx).reader_selection.text == "Alpha 中文 Beta",
            "Native mouse selection did not preserve Chinese text"
        );
        window.dispatch_action(Box::new(crate::shortcuts::CopyReaderSelection), cx);
        Ok::<_, anyhow::Error>(())
    })??;
    pause(cx).await;
    handle.update(cx, |_, _, cx| {
        anyhow::ensure!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref()
                == Some("Alpha 中文 Beta"),
            "Reader copy action did not use native selection"
        );
        Ok::<_, anyhow::Error>(())
    })??;
    screenshot(path, "windows-reader-selection", cx).await?;
    let (start, end, opens) = handle.update(cx, |_, _, cx| {
        let s = view.read(cx);
        let Some(Document::Blitz(r)) = &s.reader else {
            anyhow::bail!("HTML reader disappeared");
        };
        let link = &r.links[0];
        let origin = s.reader_surface.origin;
        Ok::<_, anyhow::Error>((
            point(
                origin.x + px(link.x + 1.),
                origin.y + px(link.y + link.height / 2.),
            ),
            point(
                origin.x + px(link.x + link.width - 1.),
                origin.y + px(link.y + link.height / 2.),
            ),
            s.actions
                .iter()
                .filter(|a| a.as_str() == "reader-link-open")
                .count(),
        ))
    })??;
    post_reader_pointer(handle, start, 0x0201, 1, cx)?;
    pause(cx).await;
    post_reader_pointer(handle, end, 0x0200, 1, cx)?;
    post_reader_pointer(handle, end, 0x0202, 0, cx)?;
    pause(cx).await;
    handle.update(cx, |_, window, cx| {
        let s = view.read(cx);
        anyhow::ensure!(
            s.actions
                .iter()
                .filter(|a| a.as_str() == "reader-link-open")
                .count()
                == opens,
            "Dragging link text activated navigation"
        );
        window.resize(original);
        Ok::<_, anyhow::Error>(())
    })??;
    pause(cx).await;
    Ok(())
}

async fn long_reader(
    view: &Entity<MailDesktop>,
    handle: AnyWindowHandle,
    path: &std::path::Path,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    handle.update(cx,|_,_,cx|view.update(cx,|s,cx| {
        s.body.as_mut().unwrap().html="<body style='margin:0'><div style='height:20000px;background:#ffe4e4'>Long header 中文</div><div style='height:640px;background:#d2f2dc;display:flex;align-items:flex-end'>Long tail 中文</div></body>".into();
        s.reader_scroll.set_offset(Point::default());s.reader_dirty=true;s.update_reader(cx);cx.notify();
    }))?;
    for _ in 0..60 {
        pause(cx).await;
        if handle.update(cx,|_,_,cx|matches!(view.read(cx).reader.as_ref(),Some(Document::Blitz(r)) if r.height>=20640.))?{break;}
    }
    handle.update(cx, |_, window, cx| {
        let s = view.read(cx);
        let Some(Document::Blitz(r)) = &s.reader else {
            anyhow::bail!("Long mail fell back instead of region rendering");
        };
        anyhow::ensure!(
            r.height >= 20640. && r.area.height <= r.viewport.height as f32 + 512.,
            "Long mail allocated a full-page image"
        );
        let focus = s.reader_focus.clone();
        window.focus(&focus, cx);
        window.dispatch_action(Box::new(crate::shortcuts::ReaderEnd), cx);
        Ok::<_, anyhow::Error>(())
    })??;
    for _ in 0..60 {
        pause(cx).await;
        if handle.update(cx,|_,_,cx|matches!(view.read(cx).reader.as_ref(),Some(Document::Blitz(r)) if r.area.y>19000.))? {break;}
    }
    handle.update(cx, |_, window, cx| {
        let s = view.read(cx);
        anyhow::ensure!(
            matches!(s.reader.as_ref(),Some(Document::Blitz(r)) if r.area.y>19000.),
            "End did not paint the long mail tail"
        );
        window.dispatch_action(Box::new(crate::shortcuts::SelectReaderAll), cx);
        Ok::<_, anyhow::Error>(())
    })??;
    for _ in 0..40 {
        pause(cx).await;
        if handle.update(cx, |_, _, cx| {
            view.read(cx)
                .reader_selection
                .text
                .contains("Long tail 中文")
        })? {
            break;
        }
    }
    handle.update(cx, |_, window, cx| {
        let text = &view.read(cx).reader_selection.text;
        anyhow::ensure!(
            text.contains("Long header 中文") && text.contains("Long tail 中文"),
            "Cross-region select-all lost text"
        );
        window.dispatch_action(Box::new(crate::shortcuts::CopyReaderSelection), cx);
        Ok::<_, anyhow::Error>(())
    })??;
    pause(cx).await;
    screenshot(path, "windows-reader-long-tail", cx).await?;
    #[cfg(windows)]
    {
        let executable = std::env::var("LIGHTMAIL_A11Y_PROBE")
            .map_err(|_| anyhow::anyhow!("Accessibility helper missing"))?;
        let result = cx
            .background_spawn(async move {
                use std::os::windows::process::CommandExt;
                std::process::Command::new(executable)
                    .arg(std::process::id().to_string())
                    .creation_flags(0x08000000)
                    .output()
            })
            .await?;
        std::fs::write(path.join("reader-accessibility.txt"), &result.stdout)?;
        anyhow::ensure!(
            result.status.success(),
            "Accessibility fixture failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        handle.update(cx, |_, window, cx| {
            anyhow::ensure!(
                view.read(cx).reader_selection.text == "Long tail 中文",
                "UIA selection did not reach the native reader"
            );
            window.dispatch_action(Box::new(crate::shortcuts::CopyReaderSelection), cx);
            Ok::<_, anyhow::Error>(())
        })??;
        pause(cx).await;
        handle.update(cx, |_, _, cx| {
            anyhow::ensure!(
                cx.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref()
                    == Some("Long tail 中文"),
                "UIA selection copied different text"
            );
            Ok::<_, anyhow::Error>(())
        })??;
    }
    handle.update(cx, |_, window, cx| {
        window.dispatch_action(Box::new(crate::shortcuts::ReaderStart), cx)
    })?;
    for _ in 0..40 {
        pause(cx).await;
        if handle.update(cx,|_,_,cx|matches!(view.read(cx).reader.as_ref(),Some(Document::Blitz(r)) if r.area.y==0.))?{break;}
    }
    handle.update(cx, |_, _, cx| {
        anyhow::ensure!(
            matches!(view.read(cx).reader.as_ref(),Some(Document::Blitz(r)) if r.area.y==0.),
            "Home did not repaint the header"
        );
        Ok::<_, anyhow::Error>(())
    })??;
    Ok(())
}

#[cfg(windows)]
async fn reader_dpi(
    view: &Entity<MailDesktop>,
    handle: AnyWindowHandle,
    path: &std::path::Path,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    let (hwnd, original, initial) = handle.update(cx, |_, window, _| {
        let hwnd = win32::hwnd(window)?;
        let mut rect = win32::DeviceRect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        anyhow::ensure!(
            unsafe { win32::GetWindowRect(hwnd, &mut rect) } != 0,
            "DPI fixture rectangle missing"
        );
        Ok::<_, anyhow::Error>((hwnd as usize, rect, window.scale_factor()))
    })??;
    for scale in [1.5f32, 2., initial] {
        let ratio = scale / initial;
        let rect = win32::DeviceRect {
            left: original.left,
            top: original.top,
            right: original.left + ((original.right - original.left) as f32 * ratio) as i32,
            bottom: original.top + ((original.bottom - original.top) as f32 * ratio) as i32,
        };
        cx.background_spawn(async move {
            let dpi = (scale * 96.).round() as usize;
            unsafe {
                win32::SendMessageW(
                    hwnd as *mut win32::c_void,
                    0x02e0,
                    dpi | dpi << 16,
                    &rect as *const _ as isize,
                )
            };
        })
        .await;
        for _ in 0..60 {
            pause(cx).await;
            if handle.update(cx,|_,window,cx| window.scale_factor()==scale&&matches!(view.read(cx).reader.as_ref(),Some(Document::Blitz(r)) if r.viewport.scale==scale))? {break;}
        }
        handle.update(cx, |_, window, cx| {
            let Some(Document::Blitz(r)) = &view.read(cx).reader else {
                anyhow::bail!("DPI transition lost HTML reader");
            };
            anyhow::ensure!(
                window.scale_factor() == scale && r.viewport.scale == scale,
                "Reader ignored native DPI transition"
            );
            let image = image::load_from_memory(&r.image.bytes)?;
            anyhow::ensure!(
                image.width() == (r.area.width * scale).ceil() as u32
                    && image.height() == (r.area.height * scale).ceil() as u32,
                "DPI surface does not match actual window scale"
            );
            Ok::<_, anyhow::Error>(())
        })??;
        screenshot(
            path,
            &format!("windows-reader-dpi-{}", (scale * 100.) as u32),
            cx,
        )
        .await?;
    }
    // Post an unmodified PageDown through this window's native keyboard path.
    handle.update(cx, |_, window, cx| {
        let focus = view.read(cx).reader_focus.clone();
        window.focus(&focus, cx);
        unsafe {
            win32::PostMessageW(hwnd as *mut win32::c_void, 0x0100, 0x22, 1);
            win32::PostMessageW(hwnd as *mut win32::c_void, 0x0101, 0x22, 1);
        }
    })?;
    for _ in 0..40 {
        pause(cx).await;
        if handle.update(cx, |_, _, cx| {
            f32::from(view.read(cx).reader_scroll.offset().y) < -10.
        })? {
            break;
        }
    }
    handle.update(cx, |_, _, cx| {
        anyhow::ensure!(
            f32::from(view.read(cx).reader_scroll.offset().y) < -10.,
            "Native PageDown did not scroll reader"
        );
        Ok::<_, anyhow::Error>(())
    })??;
    Ok(())
}

#[cfg(windows)]
async fn tray_lifecycle(
    view: &Entity<MailDesktop>,
    handle: AnyWindowHandle,
    cx: &mut AsyncApp,
) -> anyhow::Result<()> {
    use crate::tray::acceptance as tray;
    handle.update(cx, |_, window, _| tray::verify(window))??;
    handle.update(cx, |_, window, _| tray::action(window, "close"))??;
    pause(cx).await;
    handle.update(cx, |_, window, _| {
        anyhow::ensure!(
            !tray::visible(window)?,
            "Close did not hide the resident window"
        );
        Ok::<_, anyhow::Error>(())
    })??;
    let directory = handle.update(cx, |_, _, cx| view.read(cx).data_root.join("Preview"))?;
    anyhow::ensure!(
        crate::tray::activate_existing(&directory),
        "Existing instance was not found"
    );
    pause(cx).await;
    handle.update(cx, |_, window, _| {
        anyhow::ensure!(
            tray::visible(window)?,
            "Existing instance did not become visible"
        );
        Ok::<_, anyhow::Error>(())
    })??;
    for enabled in [false, true] {
        handle.update(cx, |_, window, _| tray::action(window, "automatic"))??;
        pause(cx).await;
        handle.update(cx, |_, _, cx| {
            let s = view.read(cx);
            anyhow::ensure!(
                s.service.automatic_receiving() == enabled,
                "Tray did not switch receiving mode"
            );
            anyhow::ensure!(
                s.engine
                    .setting("windows-automatic-receiving".into())?
                    .as_deref()
                    == Some(if enabled { "true" } else { "false" }),
                "Receiving preference did not persist"
            );
            Ok::<_, anyhow::Error>(())
        })??;
    }
    handle.update(cx, |_, window, _| tray::action(window, "receive"))??;
    for _ in 0..40 {
        pause(cx).await;
        if handle.update(cx, |_, _, cx| {
            view.read(cx)
                .actions
                .iter()
                .any(|action| action == "refresh")
        })? {
            break;
        }
    }
    handle.update(cx, |_, _, cx| {
        anyhow::ensure!(
            view.read(cx)
                .actions
                .iter()
                .any(|action| action == "refresh"),
            "Tray did not call shared refresh"
        );
        Ok::<_, anyhow::Error>(())
    })??;
    handle.update(cx, |_, window, _| tray::action(window, "shell-restart"))??;
    pause(cx).await;
    handle.update(cx, |_, window, _| tray::verify(window))??;
    Ok(())
}

pub fn start(view: Entity<MailDesktop>, window: &mut Window, path: PathBuf, cx: &mut App) {
    let handle = window.window_handle();
    cx.spawn(async move|cx|{
        let result=async {
            // Allow the actual platform window to finish its first layout.
            for _ in 0..8{pause(cx).await;}
            #[cfg(windows)]
            tray_lifecycle(&view,handle,cx).await?;
            click(&view,handle,"message-0",true,cx)?;
            for _ in 0..120 {
                pause(cx).await;
                if handle.update(cx,|_,_,cx| view.read(cx).reader.is_some() || view.read(cx).reader_error.is_some())? { break; }
            }
            for _ in 0..4 { pause(cx).await; }
            handle.update(cx,|_,_,cx|{let s=view.read(cx);anyhow::ensure!(s.body.is_some()&&!s.loading&&s.reader_error.is_none(),"Reader did not finish");Ok::<_,anyhow::Error>(())})??;
            let document=handle.update(cx,|_,_,cx|document_text(&view,cx))?.ok_or_else(||anyhow::anyhow!("Reader document absent"))?;
            anyhow::ensure!(document.contains("final review"),"Body text absent");
            handle.update(cx,|_,_,cx|{anyhow::ensure!(!view.read(cx).images&&blocks_remote_images(&view.read(cx).image_policy()),"Remote images not blocked by default");Ok::<_,anyhow::Error>(())})??;
            let document=serde_json::json!({"text":document});
            handle.update(cx,|_,window,cx| {
                let b = view.read(cx).probes["reader-viewport"];
                let geometry = serde_json::json!({"x":f32::from(b.left()),"y":f32::from(b.top()),"width":f32::from(b.size.width),"height":f32::from(b.size.height),"scale":window.scale_factor()});
                std::fs::write(path.join("reader-geometry.json"), geometry.to_string())
            })??;
            screenshot(&path,"windows-inbox",cx).await?;
            window_chrome(&view,handle,&path,cx).await?;
            click(&view,handle,"copy-markdown",false,cx)?;pause(cx).await;
            handle.update(cx,|_,_,cx|{anyhow::ensure!(cx.read_from_clipboard().and_then(|v|v.text()).is_some_and(|s|s.contains("final review")),"Markdown clipboard missing");Ok::<_,anyhow::Error>(())})??;
            // Prepare and check in one update: a core DataChanged event arriving later
            // reloads the cached body, so a delayed check would race it.
            let html_ready=handle.update(cx,|_,_,cx|view.update(cx,|s,cx|{
                let body=s.body.as_mut().unwrap();
                body.html=format!("<p style='font:18px Microsoft YaHei;margin:0;padding:12px'>Alpha 中文 Beta</p><table style='border:2px solid #226451'><tr><td><a href='https://example.com/synthetic-link'>Visible link</a></td></tr></table><img src='https://example.invalid/pixel.png'><script>document.body.dataset.executed='true'</script>{}",(0..80).map(|i|format!("<p>Scrollable synthetic paragraph {i}</p>")).collect::<String>());
                s.plain_reading=false;s.reader_dirty=true;s.update_reader(cx);cx.notify();
                true
            }))?;
            anyhow::ensure!(html_ready,"Restricted HTML reader content missing");
            for _ in 0..80 {
                pause(cx).await;
                if handle.update(cx,|_,_,cx|{
                    let s = view.read(cx);
                    Ok::<_,anyhow::Error>(matches!(s.reader.as_ref(),Some(Document::Blitz(rendered)) if rendered.source.contains("<table")&&rendered.source.contains("Visible link")))
                })?? { break; }
            }
            handle.update(cx,|_,_,cx|{
                let s = view.read(cx);
                anyhow::ensure!(matches!(s.reader.as_ref(),Some(Document::Blitz(rendered)) if rendered.source.contains("<table")&&rendered.source.contains("Visible link")),"Blitz reader did not finish restricted HTML");
                Ok::<_,anyhow::Error>(())
            })??;
            handle.update(cx,|_,_,cx|{
                let s=view.read(cx);
                anyhow::ensure!(blocks_remote_images(&s.image_policy()),"Remote image loaded without permission");
                anyhow::ensure!(crate::reader::allowed_link("https://example.com/synthetic-link")&&!crate::reader::allowed_link("javascript:alert(1)"),"Link policy failed");
                Ok::<_,anyhow::Error>(())
            })??;
            reader_selection(&view,handle,&path,cx).await?;
            long_reader(&view,handle,&path,cx).await?;
            #[cfg(windows)] reader_dpi(&view,handle,&path,cx).await?;
            // Opting in decodes embedded images, but never reaches local files.
            handle.update(cx,|_,_,cx|view.update(cx,|s,cx|{s.images=true;cx.notify();}))?;
            handle.update(cx,|_,_,cx|{
                let images=view.read(cx).image_policy();
                let pixel="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
                anyhow::ensure!(matches!(images(&SharedUri::from(pixel)),ImageSource::Image(_)),"Allowed embedded image not decoded");
                anyhow::ensure!(!matches!(images(&SharedUri::from("file:///C:/private.png")),ImageSource::Image(_)),"Local file image loaded");
                Ok::<_,anyhow::Error>(())
            })??;
            handle.update(cx,|_,_,cx|view.update(cx,|s,cx|{s.images=false;s.remote_images.reset();cx.notify();}))?;
            click(&view,handle,"settings",true,cx)?;pause(cx).await;
            overlay_margin(&view,handle,cx).await?;
            click(&view,handle,"settings-demo-work",true,cx)?;pause(cx).await;
            handle.update(cx,|_,_,cx|{anyhow::ensure!(view.read(cx).editing.as_ref().is_some_and(|a|a.id=="demo-work"),"Account whitespace click failed");Ok::<_,anyhow::Error>(())})??;
            screenshot(&path,"windows-settings",cx).await?;
            click(&view,handle,"translation-tab",false,cx)?;pause(cx).await;
            handle.update(cx,|_,_,cx|{anyhow::ensure!(view.read(cx).page==Page::Translation,"Translation page inaccessible");Ok::<_,anyhow::Error>(())})??;
            click(&view,handle,"storage-tab",false,cx)?;pause(cx).await;
            click(&view,handle,"settings-done",false,cx)?;pause(cx).await;
            click(&view,handle,"compose",true,cx)?;pause(cx).await;
            handle.update(cx,|_,window,cx|view.update(cx,|s,cx|{
                s.set("to","acceptance@example.com",window,cx);s.set("subject","Synthetic acceptance draft",window,cx);s.set("draft_body","Only a local draft. No real sending.",window,cx);
            }))?;pause(cx).await;
            screenshot(&path,"windows-compose",cx).await?;
            click(&view,handle,"save-draft",false,cx)?;pause(cx).await;
            handle.update(cx,|_,_,cx|{anyhow::ensure!(view.read(cx).engine.drafts()?.iter().any(|d|d.subject=="Synthetic acceptance draft"&&d.status=="draft"),"Draft did not persist");Ok::<_,anyhow::Error>(())})??;
            // Same body is rendered repeatedly to exercise renderer reuse, not just conversion.
            for i in 0..60 {handle.update(cx,|_,_,cx|view.update(cx,|s,cx|{s.page=Page::Mail;s.mode=ExportMode::Original;s.reader_dirty=true;s.record(&format!("reader-cycle-{i}"));cx.notify();}))?;pause(cx).await;}
            let platform=Arc::new(DesktopPlatform::default());let key=format!("acceptance:{}",uuid::Uuid::new_v4());platform.write_secret(key.clone(),"synthetic-only".into())?;anyhow::ensure!(platform.read_secret(key.clone())?.as_deref()==Some("synthetic-only"),"Vault roundtrip");platform.remove_secret(key)?;
            // Opt-in long run for the memory curve; synthetic demo mail only.
            let reads=std::env::var("LIGHTMAIL_SOAK_READS").ok().and_then(|v|v.parse::<usize>().ok()).unwrap_or(0);
            if reads>0 {soak(&view,handle,&path,reads,cx).await?;}
            Ok::<_,anyhow::Error>(serde_json::json!({"passed":true,"soakReads":reads,"checks":["native-window","application-icons","tray-registered","tray-hide-restore","tray-auto-receive","tray-manual-receive","tray-shell-restart","window-chrome-hit-test","window-maximize-restore","overlay-drag-margin","mail-row-whitespace","reader-native-text","reader-html-links-images","reader-pane-resize","reader-unicode-mouse-selection","reader-selection-copy","reader-drag-link-no-navigation","reader-bounded-long-document","reader-keyboard-home-end","reader-cross-region-copy","reader-uia-text-pattern","reader-uia-selected-range","reader-uia-selection-action","reader-native-dpi-transition","reader-native-page-down","remote-image-policy","image-opt-in","clipboard","account-whitespace","translation-settings","storage-settings","draft-persistence","reader-reuse-60","os-credential-roundtrip"],"document":document}))
        }.await;
        let report=match result{Ok(v)=>v,Err(e)=>serde_json::json!({"passed":false,"error":format!("{e:#}")})};
        let _=std::fs::create_dir_all(&path);let _=std::fs::write(path.join("native-acceptance.json"),report.to_string());
        // Keep the final window briefly for the CI screenshot collector, then exit normally.
        for _ in 0..20 {pause(cx).await;}
        #[cfg(windows)]
        let _=handle.update(cx,|_,window,_|crate::tray::acceptance::action(window,"exit"));
        #[cfg(not(windows))]
        let _=cx.update(|cx|cx.quit());
    }).detach();
}
