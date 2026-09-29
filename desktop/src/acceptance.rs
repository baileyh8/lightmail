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
    #[link(name = "user32")]
    extern "system" {
        pub fn PostMessageW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> i32;
        pub fn SendMessageW(hwnd: *mut c_void, msg: u32, wparam: usize, lparam: isize) -> isize;
        pub fn ClientToScreen(hwnd: *mut c_void, point: *mut DevicePoint) -> i32;
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
// The native reader has no script engine or DOM: checks read the prepared
// document and the exact image and link policy the view renders with.
fn document_text(view: &Entity<MailDesktop>, cx: &App) -> Option<String> {
    Some(match view.read(cx).reader.as_ref()? {
        Document::Markdown(text) | Document::Html(text) => text.to_string(),
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
pub fn start(view: Entity<MailDesktop>, window: &mut Window, path: PathBuf, cx: &mut App) {
    let handle = window.window_handle();
    cx.spawn(async move|cx|{
        let result=async {
            // Allow the actual platform window to finish its first layout.
            for _ in 0..8{pause(cx).await;}
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
                body.html=format!("<table style='border:2px solid #226451'><tr><td><a href='https://example.com/synthetic-link'>Visible link</a></td></tr></table><img src='https://example.invalid/pixel.png'><script>document.body.dataset.executed='true'</script>{}",(0..80).map(|i|format!("<p>Scrollable synthetic paragraph {i}</p>")).collect::<String>());
                s.plain_reading=false;s.reader_dirty=true;s.update_reader();cx.notify();
                matches!(s.reader,Some(Document::Html(ref text)) if text.contains("<table")&&text.contains("Visible link"))
            }))?;
            anyhow::ensure!(html_ready,"Restricted HTML reader content missing");
            for _ in 0..8{pause(cx).await;}
            handle.update(cx,|_,_,cx|{
                let s=view.read(cx);
                anyhow::ensure!(blocks_remote_images(&s.image_policy()),"Remote image loaded without permission");
                anyhow::ensure!(crate::reader::allowed_link("https://example.com/synthetic-link")&&!crate::reader::allowed_link("javascript:alert(1)"),"Link policy failed");
                Ok::<_,anyhow::Error>(())
            })??;
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
            Ok::<_,anyhow::Error>(serde_json::json!({"passed":true,"checks":["native-window","window-chrome-hit-test","window-maximize-restore","overlay-drag-margin","mail-row-whitespace","reader-native-text","reader-html-links-images","remote-image-policy","image-opt-in","clipboard","account-whitespace","translation-settings","storage-settings","draft-persistence","reader-reuse-60","os-credential-roundtrip"],"document":document}))
        }.await;
        let report=match result{Ok(v)=>v,Err(e)=>serde_json::json!({"passed":false,"error":format!("{e:#}")})};
        let _=std::fs::create_dir_all(&path);let _=std::fs::write(path.join("native-acceptance.json"),report.to_string());
        // Keep the final window briefly for the CI screenshot collector, then exit normally.
        for _ in 0..20 {pause(cx).await;}
        let _=cx.update(|cx|cx.quit());
    }).detach();
}
