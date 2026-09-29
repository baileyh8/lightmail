//! Opt-in native UI acceptance runner. Only compiled for CI; only uses demo data.
use crate::{
    app::{MailDesktop, Page},
    platform::DesktopPlatform,
};
use gpui::*;
use lightmail_core::{ExportMode, PlatformServices};
use std::{path::PathBuf, sync::Arc, time::Duration};

async fn pause(cx: &AsyncApp) {
    cx.background_executor()
        .timer(Duration::from_millis(250))
        .await;
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
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            #[link(name = "user32")]
            extern "system" {
                fn PostMessageW(
                    hwnd: *mut std::ffi::c_void,
                    msg: u32,
                    wparam: usize,
                    lparam: isize,
                ) -> i32;
            }
            let RawWindowHandle::Win32(raw) = HasWindowHandle::window_handle(window)?.as_raw()
            else {
                anyhow::bail!("Not a Windows window")
            };
            let x = (f32::from(position.x) * window.scale_factor()) as u16;
            let y = (f32::from(position.y) * window.scale_factor()) as u16;
            let coordinates = ((y as u32) << 16 | x as u32) as isize;
            // Target this synthetic CI window only; never move the global desktop pointer.
            unsafe {
                for (message, buttons) in [(0x0200, 0), (0x0201, 1), (0x0202, 0)] {
                    anyhow::ensure!(
                        PostMessageW(raw.hwnd.get() as *mut _, message, buttons, coordinates) != 0,
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
async fn dom(
    view: &Entity<MailDesktop>,
    handle: AnyWindowHandle,
    script: &str,
    cx: &mut AsyncApp,
) -> anyhow::Result<serde_json::Value> {
    let (tx, rx) = async_channel::bounded(1);
    let script = script.to_string();
    handle.update(cx, |_, _, cx| {
        let reader = view
            .read(cx)
            .reader
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No native reader"))?;
        reader
            .read(cx)
            .raw()
            .evaluate_script_with_callback(&script, move |v| {
                let _ = tx.try_send(v);
            })?;
        Ok::<_, anyhow::Error>(())
    })??;
    let result = rx.recv().await?;
    Ok(serde_json::from_str(&result)?)
}
pub fn start(view: Entity<MailDesktop>, window: &mut Window, path: PathBuf, cx: &mut App) {
    let handle = window.window_handle();
    cx.spawn(async move|cx|{
        let result=async {
            // Allow the actual platform window to finish its first layout.
            for _ in 0..8{pause(cx).await;}
            click(&view,handle,"message-0",true,cx)?;
            for _ in 0..20{pause(cx).await;}
            handle.update(cx,|_,_,cx|{let s=view.read(cx);anyhow::ensure!(s.body.is_some()&&!s.loading&&s.reader_error.is_none(),"Reader did not finish");Ok::<_,anyhow::Error>(())})??;
            let document=dom(&view,handle,"JSON.stringify({text:document.body.innerText,links:document.links.length,csp:document.querySelector('meta[http-equiv]').content})",cx).await?;
            let document:serde_json::Value=serde_json::from_str(document.as_str().ok_or_else(||anyhow::anyhow!("Reader JSON"))?)?;
            anyhow::ensure!(document["text"].as_str().is_some_and(|s|s.contains("final review")),"Body text absent");
            anyhow::ensure!(document["csp"].as_str().is_some_and(|s|s.contains("img-src 'none'")),"Remote images not blocked");
            click(&view,handle,"copy-markdown",false,cx)?;pause(cx).await;
            handle.update(cx,|_,_,cx|{anyhow::ensure!(cx.read_from_clipboard().and_then(|v|v.text()).is_some_and(|s|s.contains("final review")),"Markdown clipboard missing");Ok::<_,anyhow::Error>(())})??;
            handle.update(cx,|_,_,cx|view.update(cx,|s,cx|{
                let body=s.body.as_mut().unwrap();
                body.html=format!("<table style='border:2px solid #226451'><tr><td><a href='https://example.com/synthetic-link'>Visible link</a></td></tr></table><img src='https://example.invalid/pixel.png'><script>document.body.dataset.executed='true'</script>{}",(0..80).map(|i|format!("<p>Scrollable synthetic paragraph {i}</p>")).collect::<String>());
                s.reader_dirty=true;cx.notify();
            }))?;
            for _ in 0..8{pause(cx).await;}
            let render=dom(&view,handle,"JSON.stringify({links:document.links.length,table:!!document.querySelector('table'),executed:document.body.dataset.executed||'',image:getComputedStyle(document.querySelector('img')).display,scroll:(window.scrollTo(0,document.body.scrollHeight),window.scrollY)})",cx).await?;
            let render:serde_json::Value=serde_json::from_str(render.as_str().ok_or_else(||anyhow::anyhow!("Layout JSON"))?)?;
            anyhow::ensure!(render["links"]==1&&render["table"]==true&&render["executed"]==""&&render["image"]=="none"&&render["scroll"].as_f64().is_some_and(|y|y>100.),"Reader layout, script policy or scrolling failed: {render}");
            click(&view,handle,"settings",true,cx)?;pause(cx).await;
            click(&view,handle,"settings-demo-work",true,cx)?;pause(cx).await;
            handle.update(cx,|_,_,cx|{anyhow::ensure!(view.read(cx).editing.as_ref().is_some_and(|a|a.id=="demo-work"),"Account whitespace click failed");Ok::<_,anyhow::Error>(())})??;
            click(&view,handle,"translation-tab",false,cx)?;pause(cx).await;
            handle.update(cx,|_,_,cx|{anyhow::ensure!(view.read(cx).page==Page::Translation,"Translation page inaccessible");Ok::<_,anyhow::Error>(())})??;
            click(&view,handle,"storage-tab",false,cx)?;pause(cx).await;
            click(&view,handle,"settings-done",false,cx)?;pause(cx).await;
            click(&view,handle,"compose",true,cx)?;pause(cx).await;
            handle.update(cx,|_,window,cx|view.update(cx,|s,cx|{
                s.set("to","acceptance@example.com",window,cx);s.set("subject","Synthetic acceptance draft",window,cx);s.set("draft_body","Only a local draft. No real sending.",window,cx);
            }))?;pause(cx).await;
            click(&view,handle,"save-draft",false,cx)?;pause(cx).await;
            handle.update(cx,|_,_,cx|{anyhow::ensure!(view.read(cx).engine.drafts()?.iter().any(|d|d.subject=="Synthetic acceptance draft"&&d.status=="draft"),"Draft did not persist");Ok::<_,anyhow::Error>(())})??;
            // Same body is rendered repeatedly to exercise renderer reuse, not just conversion.
            for i in 0..60 {handle.update(cx,|_,_,cx|view.update(cx,|s,cx|{s.page=Page::Mail;s.mode=ExportMode::Original;s.reader_dirty=true;s.record(&format!("reader-cycle-{i}"));cx.notify();}))?;pause(cx).await;}
            let platform=Arc::new(DesktopPlatform::default());let key=format!("acceptance:{}",uuid::Uuid::new_v4());platform.write_secret(key.clone(),"synthetic-only".into())?;anyhow::ensure!(platform.read_secret(key.clone())?.as_deref()==Some("synthetic-only"),"Vault roundtrip");platform.remove_secret(key)?;
            Ok::<_,anyhow::Error>(serde_json::json!({"passed":true,"checks":["native-window","mail-row-whitespace","reader-dom","reader-layout-links-scroll","remote-image-policy","clipboard","account-whitespace","translation-settings","storage-settings","draft-persistence","reader-reuse-60","os-credential-roundtrip"],"document":document}))
        }.await;
        let report=match result{Ok(v)=>v,Err(e)=>serde_json::json!({"passed":false,"error":format!("{e:#}")})};
        let _=std::fs::create_dir_all(&path);let _=std::fs::write(path.join("native-acceptance.json"),report.to_string());
        // Keep the final window briefly for the CI screenshot collector, then exit normally.
        for _ in 0..20 {pause(cx).await;}
        let _=cx.update(|cx|cx.quit());
    }).detach();
}
