//! Standalone Kit HTML preview for local research. Does not construct a mail
//! engine/service or load images. The caller chooses the input and output files.
use gpui_kit::{
    base::{TextView, TextViewState},
    prelude::*,
    *,
};
use std::{path::PathBuf, time::Duration};
#[path = "../src/assets.rs"]
mod assets;

struct Preview {
    document: Entity<TextViewState>,
}
impl Render for Preview {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().bg(rgb(0xffffff)).child(
            TextView::new(&self.document)
                .scrollable(true)
                .size_full()
                .text_size(px(15.))
                .line_height(relative(1.65))
                .image_source(|_| ImageSource::from("icons/image-off.svg")),
        )
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let source =
        std::fs::read_to_string(args.first().expect("Pass HTML file")).expect("Read input");
    let output = PathBuf::from(args.get(1).expect("Pass output prefix"));
    let width: f32 = args
        .get(2)
        .map(|s| s.parse().expect("Width"))
        .unwrap_or(720.);
    gpui_kit::application().with_assets(assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        let (handle, document) = gpui_kit::open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.),px(0.)),size(px(width),px(640.))))),
            focus:false, show:true, ..Default::default()
        },cx,|_,cx| {
            let document = cx.new(|cx|TextViewState::html(&source,cx));
            cx.new(|_|Preview {document})
        }).expect("Open research preview");
        cx.spawn(async move |cx| {
            for _ in 0..12 { cx.background_executor().timer(Duration::from_millis(100)).await; }
            let _ = handle.update(cx,|_,window,cx| {
                let state = document.read(cx);
                let text = state.document.read(cx).rendered_text();
                let report = serde_json::json!({"inputBytes":source.len(),"renderedTextBytes":text.as_str().len(),"width":width});
                std::fs::write(output.with_extension("json"),report.to_string()).expect("Write metrics");
                // Only this unfocused probe HWND is captured. A visible D3D
                // surface is required; hidden surfaces yield black PrintWindow.
                capture(window,&output.with_extension("png"));
            });
            let _ = cx.update(|cx|cx.quit());
        }).detach();
    });
}

#[cfg(windows)]
fn capture(window: &Window, output: &std::path::Path) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::c_void;
    #[repr(C)]
    struct Rect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }
    #[repr(C)]
    struct BitmapInfo {
        size: u32,
        width: i32,
        height: i32,
        planes: u16,
        bits: u16,
        compression: u32,
        bytes: u32,
        x: i32,
        y: i32,
        colors: u32,
        important: u32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetClientRect(h: *mut c_void, r: *mut Rect) -> i32;
        fn GetDC(h: *mut c_void) -> *mut c_void;
        fn ReleaseDC(h: *mut c_void, dc: *mut c_void) -> i32;
        fn PrintWindow(h: *mut c_void, dc: *mut c_void, flags: u32) -> i32;
    }
    #[link(name = "gdi32")]
    extern "system" {
        fn CreateCompatibleDC(dc: *mut c_void) -> *mut c_void;
        fn CreateCompatibleBitmap(dc: *mut c_void, w: i32, h: i32) -> *mut c_void;
        fn SelectObject(dc: *mut c_void, obj: *mut c_void) -> *mut c_void;
        fn DeleteDC(dc: *mut c_void) -> i32;
        fn DeleteObject(obj: *mut c_void) -> i32;
        fn GetDIBits(
            dc: *mut c_void,
            b: *mut c_void,
            start: u32,
            lines: u32,
            pixels: *mut c_void,
            info: *mut BitmapInfo,
            usage: u32,
        ) -> i32;
    }
    let RawWindowHandle::Win32(raw) = HasWindowHandle::window_handle(window).unwrap().as_raw()
    else {
        return;
    };
    unsafe {
        let h = raw.hwnd.get() as *mut c_void;
        let mut rect = Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        GetClientRect(h, &mut rect);
        let (w, height) = (rect.right, rect.bottom);
        if w <= 0 || height <= 0 {
            return;
        }
        let dc = GetDC(h);
        let memory = CreateCompatibleDC(dc);
        let bitmap = CreateCompatibleBitmap(dc, w, height);
        let previous = SelectObject(memory, bitmap);
        let ok = PrintWindow(h, memory, 3);
        SelectObject(memory, previous);
        let mut pixels = vec![0u8; w as usize * height as usize * 4];
        let mut info = BitmapInfo {
            size: 40,
            width: w,
            height: -height,
            planes: 1,
            bits: 32,
            compression: 0,
            bytes: 0,
            x: 0,
            y: 0,
            colors: 0,
            important: 0,
        };
        let lines = GetDIBits(
            memory,
            bitmap,
            0,
            height as u32,
            pixels.as_mut_ptr().cast(),
            &mut info,
            0,
        );
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(h, dc);
        if ok != 0 && lines != 0 {
            // Save as BMP without adding an imaging dependency to the app.
            let mut bmp = Vec::with_capacity(54 + pixels.len());
            bmp.extend(b"BM");
            bmp.extend(((54 + pixels.len()) as u32).to_le_bytes());
            bmp.extend([0u8; 4]);
            bmp.extend(54u32.to_le_bytes());
            bmp.extend(std::slice::from_raw_parts(
                &info as *const _ as *const u8,
                40,
            ));
            bmp.extend(pixels);
            std::fs::write(output.with_extension("bmp"), bmp).expect("Write preview");
        }
    }
}
#[cfg(not(windows))]
fn capture(_: &Window, _: &std::path::Path) {}
