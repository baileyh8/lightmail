//! Same Win32 host, either one reused WebView2 control or the current Blitz
//! full-page raster -> PNG -> decode pipeline. No mail engine or credentials.
use anyhow::{Result, bail};
use anyrender::ImageRenderer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{DocumentConfig, StyleThreading};
use blitz_html::HtmlDocument;
use blitz_traits::{
    net::DummyNetProvider,
    shell::{ColorScheme, Viewport},
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{
            Com::{StructuredStorage::CreateStreamOnHGlobal, *},
            Diagnostics::ToolHelp::*,
            LibraryLoader::*,
            ProcessStatus::*,
            Threading::*,
        },
        UI::{HiDpi::*, WindowsAndMessaging::*},
    },
    core::{BOOL, Interface, PCWSTR, PWSTR, w},
};

const WIDTH: u32 = 720;
const HEIGHT: u32 = 640;

struct Page {
    bgra: Vec<u8>,
    height: u32,
}
thread_local! { static PAGE: RefCell<Option<Page>> = const { RefCell::new(None) }; }
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let dc = unsafe { BeginPaint(hwnd, &mut ps) };
            PAGE.with(|page| {
                if let Some(page) = page.borrow().as_ref() {
                    let mut info = BITMAPINFO::default();
                    info.bmiHeader = BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: WIDTH as i32,
                        biHeight: -(page.height as i32),
                        biPlanes: 1,
                        biBitCount: 32,
                        ..Default::default()
                    };
                    unsafe {
                        StretchDIBits(
                            dc,
                            0,
                            0,
                            WIDTH as i32,
                            page.height as i32,
                            0,
                            0,
                            WIDTH as i32,
                            page.height as i32,
                            Some(page.bgra.as_ptr().cast()),
                            &info,
                            DIB_RGB_COLORS,
                            SRCCOPY,
                        );
                    }
                }
            });
            let _ = unsafe { EndPaint(hwnd, &ps) };
            LRESULT(0)
        }
        WM_CLOSE => {
            let _ = unsafe { DestroyWindow(hwnd) };
            LRESULT(0)
        }
        WM_DESTROY => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

fn window() -> Result<HWND> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let module = GetModuleHandleW(None)?;
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: HINSTANCE(module.0),
            lpszClassName: w!("Lightmail.ReaderPerformanceProbe"),
            hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            bail!(
                "Register probe window: {}",
                windows::core::Error::from_win32()
            );
        }
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: WIDTH as i32,
            bottom: HEIGHT as i32,
        };
        AdjustWindowRectEx(&mut rect, WS_OVERLAPPEDWINDOW, false, WS_EX_NOACTIVATE)?;
        let handle = CreateWindowExW(
            WS_EX_NOACTIVATE,
            class.lpszClassName,
            w!("轻邮阅读器资源对照 · 独立测试"),
            WS_OVERLAPPEDWINDOW,
            60,
            60,
            rect.right - rect.left,
            rect.bottom - rect.top,
            None,
            None,
            Some(HINSTANCE(module.0)),
            None,
        )?;
        let _ = ShowWindow(handle, SW_SHOWNOACTIVATE);
        Ok(handle)
    }
}

fn pump() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
fn settle(ms: u64) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(ms) {
        pump();
        thread::sleep(Duration::from_millis(5));
    }
}
fn receive<T>(rx: &mpsc::Receiver<T>) -> Result<T> {
    let start = Instant::now();
    loop {
        if let Ok(value) = rx.try_recv() {
            return Ok(value);
        }
        if start.elapsed() > Duration::from_secs(30) {
            bail!("WebView2 operation timed out");
        }
        pump();
        thread::sleep(Duration::from_millis(2));
    }
}

#[derive(Clone)]
struct ProcessMetric {
    private: u64,
    working: u64,
    cpu_ms: f64,
}
fn filetime(value: FILETIME) -> u64 {
    (value.dwHighDateTime as u64) << 32 | value.dwLowDateTime as u64
}
fn processes() -> HashMap<u32, ProcessMetric> {
    unsafe {
        let mut tree = HashMap::new();
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
            return tree;
        };
        let mut e = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut parents = vec![];
        if Process32FirstW(snapshot, &mut e).is_ok() {
            loop {
                parents.push((e.th32ProcessID, e.th32ParentProcessID));
                if Process32NextW(snapshot, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
        let mut ids = HashSet::from([GetCurrentProcessId()]);
        loop {
            let before = ids.len();
            for &(id, parent) in &parents {
                if ids.contains(&parent) {
                    ids.insert(id);
                }
            }
            if before == ids.len() {
                break;
            }
        }
        for id in ids {
            let Ok(h) = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, id) else {
                continue;
            };
            let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
            let result = GetProcessMemoryInfo(
                h,
                &mut counters as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            );
            let (mut created, mut exited, mut kernel, mut user) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            let _ = GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user);
            let _ = CloseHandle(h);
            if result.is_ok() {
                tree.insert(
                    id,
                    ProcessMetric {
                        private: counters.PrivateUsage as u64,
                        working: counters.WorkingSetSize as u64,
                        cpu_ms: (filetime(kernel) + filetime(user)) as f64 / 10000.,
                    },
                );
            }
        }
        tree
    }
}
struct Monitor {
    state: Arc<Mutex<(u64, u64, usize, HashMap<u32, f64>)>>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Monitor {
    fn observe_host(&self) {
        // Short native allocations can occur between 100ms tree samples.
        // Observe live buffers at native pipeline boundaries as well.
        unsafe {
            let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
            if GetProcessMemoryInfo(
                GetCurrentProcess(),
                &mut counters as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            )
            .is_ok()
            {
                let mut state = self.state.lock().unwrap();
                state.0 = state.0.max(counters.PrivateUsage as u64);
                state.1 = state.1.max(counters.WorkingSetSize as u64);
            }
        }
    }
    fn start() -> Self {
        let state = Arc::new(Mutex::new((0, 0, 0, HashMap::new())));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st) = (state.clone(), stop.clone());
        let handle = thread::spawn(move || {
            while !st.load(Ordering::Relaxed) {
                let p = processes();
                let mut s = s.lock().unwrap();
                s.0 = s.0.max(p.values().map(|p| p.private).sum());
                s.1 = s.1.max(p.values().map(|p| p.working).sum());
                s.2 = s.2.max(p.len());
                for (id, v) in p {
                    s.3.insert(id, v.cpu_ms);
                }
                drop(s);
                thread::sleep(Duration::from_millis(100));
            }
        });
        Self {
            state,
            stop,
            thread: Some(handle),
        }
    }
    fn metric(&self) -> Value {
        let p = processes();
        let s = self.state.lock().unwrap();
        let host = unsafe { GetCurrentProcessId() };
        json!({"processCount":p.len(),"hostPrivateMiB":p.get(&host).map(|p|p.private as f64/1048576.),
            "treePrivateMiB":p.values().map(|p|p.private).sum::<u64>() as f64/1048576.,
            "treeWorkingSetMiB":p.values().map(|p|p.working).sum::<u64>() as f64/1048576.,
            "peakTreePrivateMiB":s.0 as f64/1048576.,"peakTreeWorkingSetMiB":s.1 as f64/1048576.,
            "peakProcesses":s.2,"treeCpuMs":s.3.values().sum::<f64>()})
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Browser {
    controller: ICoreWebView2Controller,
    view: ICoreWebView2,
    environment: ICoreWebView2Environment,
    blocked: Arc<Mutex<u64>>,
    version: String,
}
impl Browser {
    fn create(hwnd: HWND, data: &Path) -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let handler =
            CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |result, env| {
                tx.send(
                    result.and_then(|_| env.ok_or_else(|| windows::core::Error::from(E_POINTER))),
                )
                .unwrap();
                Ok(())
            }));
        let user = CoTaskMemPWSTR::from(data.to_string_lossy().as_ref());
        unsafe {
            CreateCoreWebView2EnvironmentWithOptions(
                PCWSTR::null(),
                *user.as_ref().as_pcwstr(),
                None,
                &handler,
            )?;
        }
        let environment = receive(&rx)??;
        let (tx, rx) = mpsc::channel();
        let handler =
            CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |result, ctrl| {
                tx.send(
                    result.and_then(|_| ctrl.ok_or_else(|| windows::core::Error::from(E_POINTER))),
                )
                .unwrap();
                Ok(())
            }));
        unsafe {
            environment.CreateCoreWebView2Controller(hwnd, &handler)?;
        }
        let controller = receive(&rx)??;
        let view = unsafe { controller.CoreWebView2()? };
        unsafe {
            controller.SetBounds(RECT {
                left: 0,
                top: 0,
                right: WIDTH as i32,
                bottom: HEIGHT as i32,
            })?;
            controller.SetIsVisible(true)?;
            let controller2: ICoreWebView2Controller2 = controller.cast()?;
            controller2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR {
                A: 255,
                R: 255,
                G: 255,
                B: 255,
            })?;
            let view13: ICoreWebView2_13 = view.cast()?;
            view13
                .Profile()?
                .SetPreferredColorScheme(COREWEBVIEW2_PREFERRED_COLOR_SCHEME_LIGHT)?;
            let settings = view.Settings()?;
            settings.SetIsScriptEnabled(false)?;
            settings.SetAreDevToolsEnabled(false)?;
            settings.SetAreHostObjectsAllowed(false)?;
            settings.SetIsWebMessageEnabled(false)?;
            settings.SetAreDefaultScriptDialogsEnabled(false)?;
            settings.SetAreDefaultContextMenusEnabled(false)?;
            view.AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)?;
        }
        let blocked = Arc::new(Mutex::new(0));
        let (requests, env) = (blocked.clone(), environment.clone());
        let handler = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
            if let Some(args) = args {
                unsafe {
                    let response = env.CreateWebResourceResponse(
                        None,
                        403,
                        w!("Blocked"),
                        w!("Content-Length: 0"),
                    )?;
                    args.SetResponse(&response)?;
                }
                *requests.lock().unwrap() += 1;
            }
            Ok(())
        }));
        let mut token = 0;
        unsafe {
            view.add_WebResourceRequested(&handler, &mut token)?;
        }
        let mut v = PWSTR::null();
        unsafe {
            environment.BrowserVersionString(&mut v)?;
        }
        let version = CoTaskMemPWSTR::from(v).to_string();
        Ok(Self {
            controller,
            view,
            environment,
            blocked,
            version,
        })
    }
    fn load(&self, html: &str) -> Result<f64> {
        let (tx, rx) = mpsc::channel();
        let handler = NavigationCompletedEventHandler::create(Box::new(move |_, args| {
            let mut ok = BOOL(0);
            if let Some(args) = args {
                unsafe {
                    args.IsSuccess(&mut ok)?;
                }
            }
            let _ = tx.send(ok.as_bool());
            Ok(())
        }));
        let mut token = 0;
        unsafe {
            self.view.add_NavigationCompleted(&handler, &mut token)?;
        }
        let content = CoTaskMemPWSTR::from(html);
        let start = Instant::now();
        unsafe {
            self.view.NavigateToString(*content.as_ref().as_pcwstr())?;
        }
        let ok = receive(&rx)?;
        unsafe {
            self.view.remove_NavigationCompleted(token)?;
        }
        if !ok {
            bail!("WebView2 navigation failed");
        }
        Ok(start.elapsed().as_secs_f64() * 1000.)
    }
    fn suspend(&self) -> Result<bool> {
        unsafe {
            self.controller.SetIsVisible(false)?;
        }
        let view: ICoreWebView2_3 = self.view.cast()?;
        let (tx, rx) = mpsc::channel();
        let handler = TrySuspendCompletedHandler::create(Box::new(move |status, ok| {
            let _ = tx.send(status.is_ok() && ok);
            Ok(())
        }));
        unsafe {
            view.TrySuspend(&handler)?;
        }
        receive(&rx)
    }
    fn capture(&self, path: &Path) -> Result<()> {
        let stream = unsafe { CreateStreamOnHGlobal(HGLOBAL::default(), true)? };
        let (tx, rx) = mpsc::channel();
        let handler = CapturePreviewCompletedHandler::create(Box::new(move |status| {
            let _ = tx.send(status);
            Ok(())
        }));
        unsafe {
            self.view.CapturePreview(
                COREWEBVIEW2_CAPTURE_PREVIEW_IMAGE_FORMAT_PNG,
                &stream,
                &handler,
            )?;
        }
        receive(&rx)??;
        let mut stat = STATSTG::default();
        unsafe {
            stream.Stat(&mut stat, STATFLAG_NONAME)?;
            stream.Seek(0, STREAM_SEEK_SET, None)?;
        }
        if stat.cbSize > 32 * 1024 * 1024 {
            bail!("Unexpected preview size");
        }
        let mut bytes = vec![0; stat.cbSize as usize];
        let mut read = 0;
        unsafe {
            stream
                .Read(
                    bytes.as_mut_ptr().cast(),
                    bytes.len() as u32,
                    Some(&mut read),
                )
                .ok()?;
        }
        bytes.truncate(read as usize);
        // Decoder verifies the operation returned an actual image at the agreed viewport.
        let preview = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)?;
        if preview.width() != WIDTH || preview.height() != HEIGHT {
            bail!("Unexpected preview viewport");
        }
        std::fs::write(path, bytes)?;
        Ok(())
    }
    fn processes(&self) -> Result<usize> {
        let env: ICoreWebView2Environment8 = self.environment.cast()?;
        let list = unsafe { env.GetProcessInfos()? };
        let mut count = 0;
        unsafe {
            list.Count(&mut count)?;
        }
        Ok(count as usize)
    }
}
impl Drop for Browser {
    fn drop(&mut self) {
        let _ = unsafe { self.controller.Close() };
    }
}

fn raster(hwnd: HWND, html: &str, monitor: &Monitor) -> Result<f64> {
    let start = Instant::now();
    let mut doc = HtmlDocument::from_html(
        html,
        DocumentConfig {
            viewport: Some(Viewport::new(WIDTH, HEIGHT, 1., ColorScheme::Light)),
            net_provider: Some(Arc::new(DummyNetProvider)),
            style_threading: StyleThreading::Sequential,
            ..Default::default()
        },
    );
    for _ in 0..3 {
        doc.resolve(0.);
    }
    let root = doc.root_element().final_layout();
    let height = root
        .size
        .height
        .max(root.scrollable_overflow_rect.bottom)
        .ceil()
        .max(HEIGHT as f32);
    if !height.is_finite() || height > 16000. {
        bail!("Blitz document exceeds current product bound");
    }
    let height = height as u32;
    let mut renderer = VelloCpuImageRenderer::new(WIDTH, height);
    let mut pixels = vec![];
    renderer.render_to_vec(
        |scene| blitz_paint::paint_scene(scene, &mut doc, 1., WIDTH, height, 0, 0),
        &mut pixels,
    );
    monitor.observe_host();
    for pixel in pixels.chunks_exact_mut(4) {
        let bg = 255 - pixel[3] as u16;
        for c in &mut pixel[..3] {
            *c = (*c as u16 + bg).min(255) as u8;
        }
        pixel[3] = 255;
    }
    // Include the same PNG encoding and decoding cost as today's product adapter.
    let mut png = vec![];
    let mut encoder = png::Encoder::new(&mut png, WIDTH, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&pixels)?;
    monitor.observe_host();
    let mut bgra = image::load_from_memory_with_format(&png, image::ImageFormat::Png)?
        .into_rgba8()
        .into_raw();
    monitor.observe_host();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    PAGE.with(|page| *page.borrow_mut() = Some(Page { bgra, height }));
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, true);
        let _ = UpdateWindow(hwnd);
    }
    Ok(start.elapsed().as_secs_f64() * 1000.)
}

fn wrap(html: &str) -> String {
    // Identical offline document restriction for both engines. Page scripts,
    // tracking pixels, stylesheet/font downloads and subframes are blocked.
    let csp = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; img-src 'none'; frame-src 'none'; connect-src 'none'; form-action 'none'; base-uri 'none'\">";
    if html.contains("<head>") {
        html.replacen("<head>", &format!("<head>{csp}"), 1)
    } else {
        format!("<!doctype html><html><head>{csp}</head><body>{html}</body></html>")
    }
}
fn capture_raster(path: &Path) -> Result<()> {
    PAGE.with(|page| -> Result<()> {
        let page = page.borrow();
        let page = page
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No rendered page"))?;
        let mut pixels = page.bgra[..(WIDTH * HEIGHT * 4) as usize].to_vec();
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        image::save_buffer_with_format(
            path,
            &pixels,
            WIDTH,
            HEIGHT,
            image::ColorType::Rgba8,
            image::ImageFormat::Png,
        )?;
        Ok(())
    })
}
fn main() -> Result<()> {
    let clock = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let capture = args.len() == 4 && args[3] == "--capture-check";
    if args.len() != 3 && !capture {
        bail!(
            "Usage: reader-performance-probe webview2|blitz PRIVATE_INPUT BUILD_OUTPUT [--capture-check]"
        );
    }
    let mode = &args[0];
    if !matches!(mode.as_str(), "webview2" | "blitz") {
        bail!("Unknown engine");
    }
    let output = Path::new(&args[2]);
    std::fs::create_dir_all(output)?;
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    if !output
        .canonicalize()?
        .starts_with(workspace.join("build").canonicalize()?)
    {
        bail!("Metrics/profile must stay in build/");
    }
    let mut files = std::fs::read_dir(&args[1])?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|s| s == "html"))
        .collect::<Vec<_>>();
    files.sort();
    if files.len() < 20 {
        bail!("Need twenty cached samples");
    }
    let htmls = files
        .iter()
        .take(20)
        .map(|p| std::fs::read_to_string(p).map(|s| wrap(&s)))
        .collect::<std::io::Result<Vec<_>>>()?;
    let monitor = Monitor::start();
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
    }
    let hwnd = window()?;
    let dpi = unsafe { GetDpiForWindow(hwnd) };
    if dpi != 96 {
        bail!("Run the matched 720px probe on a monitor with 100% scale (96 DPI)");
    }
    settle(1000);
    let baseline = monitor.metric();
    let started = Instant::now();
    let browser = if mode == "webview2" {
        Some(Browser::create(hwnd, &output.join("runtime-profile"))?)
    } else {
        None
    };
    let init_ms = started.elapsed().as_secs_f64() * 1000.;
    let simple = wrap(
        "<html><head></head><body style='margin:0;font:15px Segoe UI'>Synthetic initial document.</body></html>",
    );
    let simple_ms = if let Some(b) = &browser {
        b.load(&simple)?
    } else {
        raster(hwnd, &simple, &monitor)?
    };
    let first_document_ms = clock.elapsed().as_secs_f64() * 1000. - 1000.;
    settle(1500);
    let simple_memory = monitor.metric();
    let mut reads = vec![];
    for (index, html) in htmls.iter().enumerate() {
        let ms = if let Some(b) = &browser {
            b.load(html)?
        } else {
            raster(hwnd, html, &monitor)?
        };
        settle(200);
        reads.push(json!({"sample":index+1,"loadMs":ms,"metrics":monitor.metric()}));
        if capture && matches!(index, 0 | 19) {
            let path = output.join(format!("private-preview-{:02}.png", index + 1));
            if let Some(b) = &browser {
                b.capture(&path)?;
            } else {
                capture_raster(&path)?;
            }
        }
    }
    let last_document = monitor.metric();
    settle(3000);
    let idle = monitor.metric();
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    let suspended = if let Some(b) = &browser {
        Some(b.suspend()?)
    } else {
        PAGE.with(|p| p.borrow_mut().take());
        None
    };
    settle(3000);
    let hidden = monitor.metric();
    let runtime_version = browser.as_ref().map(|b| b.version.clone());
    let runtime_processes = browser.as_ref().map(|b| b.processes()).transpose()?;
    let blocked = browser.as_ref().map(|b| *b.blocked.lock().unwrap());
    drop(browser);
    PAGE.with(|p| p.borrow_mut().take());
    settle(3000);
    let released = monitor.metric();
    let report = json!({"engine":mode,"viewport":[WIDTH,HEIGHT],"dpi":dpi,"colorScheme":"light","initMs":init_ms,"simpleLoadMs":simple_ms,"readerReadyMs":init_ms+simple_ms,"captureCheck":capture,
        "firstDocumentMsExcludingBaselineWait":first_document_ms,"baseline":baseline,"simple":simple_memory,
        "reads":reads,"afterReads":last_document,"idle":idle,"hidden":hidden,"released":released,
        "suspendSuccessful":suspended,"runtimeVersion":runtime_version,"runtimeProcessInfoCount":runtime_processes,"blockedDocumentRequests":blocked,
        "htmlPolicy":"Cached sanitized HTML, scripts/images/fonts/CSS downloads blocked",
        "timingBoundary":"WebView2 NavigateToString -> NavigationCompleted; Blitz parse/layout/full CPU raster/PNG encode+decode+GDI paint; not matched presentation timestamps",
        "memoryBoundary":"Host and descendants; 100ms tree samples plus native live-buffer observations, peaks are lower bounds; private commit distinct from summed working set (shared pages may double-count); excludes dedicated GPU memory"});
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    unsafe {
        DestroyWindow(hwnd)?;
        CoUninitialize();
    }
    println!("{mode}: twenty samples completed; capture check: {capture}");
    Ok(())
}
