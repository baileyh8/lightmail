//! Isolated layout probe. No browser, window, HTTP client or real mailbox.
use litehtml::{pixbuf::PixbufContainer, selection::Selection, *};
use serde_json::{json, Value};
use std::{cell::RefCell, collections::BTreeMap, path::Path, rc::Rc, time::Instant};

#[derive(Default)]
struct Trace {
    text: Vec<Value>,
    solid: Vec<Value>,
    image_callbacks: Vec<String>,
    css_callbacks: Vec<String>,
    clicked: Option<String>,
    measured: usize,
    fonts: usize,
}

struct Container {
    pixels: PixbufContainer,
    trace: Rc<RefCell<Trace>>,
}

fn pos(p: Position) -> Value {
    json!({"x":p.x,"y":p.y,"width":p.width,"height":p.height})
}

#[cfg(windows)]
fn memory() -> Value {
    use windows_sys::Win32::System::{ProcessStatus::*, Threading::GetCurrentProcess};
    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    let bytes = std::mem::size_of_val(&counters) as u32;
    if unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
            bytes,
        )
    } == 0
    {
        return Value::Null;
    }
    json!({"privateBytes":counters.PrivateUsage,"workingSetBytes":counters.WorkingSetSize,"peakWorkingSetBytes":counters.PeakWorkingSetSize})
}
#[cfg(not(windows))]
fn memory() -> Value {
    Value::Null
}

impl DocumentContainer for Container {
    fn create_font(&mut self, d: &FontDescription) -> (FontHandle, FontMetrics) {
        self.trace.borrow_mut().fonts += 1;
        self.pixels.create_font(d)
    }
    fn delete_font(&mut self, f: FontHandle) {
        self.pixels.delete_font(f);
    }
    fn text_width(&self, text: &str, f: FontHandle) -> f32 {
        self.trace.borrow_mut().measured += 1;
        self.pixels.text_width(text, f)
    }
    fn draw_text(&mut self, h: DrawContext, text: &str, f: FontHandle, color: Color, p: Position) {
        self.trace.borrow_mut().text.push(json!({"text":text,"font":f.0,"bounds":pos(p),"color":[color.r,color.g,color.b,color.a]}));
        self.pixels.draw_text(h, text, f, color, p);
    }
    fn default_font_name(&self) -> &str {
        "sans-serif"
    }
    fn draw_list_marker(&mut self, h: DrawContext, m: &ListMarker) {
        self.pixels.draw_list_marker(h, m);
    }
    fn load_image(&mut self, src: &str, _: &str, _: bool) {
        // Record only. Never fetch, including local file and CSS background URLs.
        self.trace.borrow_mut().image_callbacks.push(src.into());
    }
    fn get_image_size(&self, _: &str, _: &str) -> Size {
        Size::default()
    }
    fn draw_image(&mut self, _: DrawContext, _: &BackgroundLayer, _: &str, _: &str) {}
    fn draw_solid_fill(&mut self, h: DrawContext, layer: &BackgroundLayer, color: Color) {
        self.trace.borrow_mut().solid.push(
            json!({"bounds":pos(layer.border_box()),"color":[color.r,color.g,color.b,color.a]}),
        );
        self.pixels.draw_solid_fill(h, layer, color);
    }
    fn draw_linear_gradient(&mut self, h: DrawContext, l: &BackgroundLayer, g: &LinearGradient) {
        self.pixels.draw_linear_gradient(h, l, g);
    }
    fn draw_radial_gradient(&mut self, h: DrawContext, l: &BackgroundLayer, g: &RadialGradient) {
        self.pixels.draw_radial_gradient(h, l, g);
    }
    fn draw_conic_gradient(&mut self, h: DrawContext, l: &BackgroundLayer, g: &ConicGradient) {
        self.pixels.draw_conic_gradient(h, l, g);
    }
    fn draw_borders(&mut self, h: DrawContext, b: &Borders, p: Position, root: bool) {
        self.pixels.draw_borders(h, b, p, root);
    }
    fn on_anchor_click(&mut self, url: &str) {
        self.trace.borrow_mut().clicked = Some(url.into());
    }
    fn transform_text(&self, text: &str, t: TextTransform) -> String {
        self.pixels.transform_text(text, t)
    }
    fn import_css(&self, url: &str, _: &str) -> (String, Option<String>) {
        self.trace.borrow_mut().css_callbacks.push(url.into());
        (String::new(), None)
    }
    fn set_clip(&mut self, p: Position, r: BorderRadiuses) {
        self.pixels.set_clip(p, r);
    }
    fn del_clip(&mut self) {
        self.pixels.del_clip();
    }
    fn get_viewport(&self) -> Position {
        self.pixels.get_viewport()
    }
    fn get_media_features(&self) -> MediaFeatures {
        self.pixels.get_media_features()
    }
}

fn placement(doc: &Document<'_>, id: &str) -> Option<Position> {
    Some(doc.root()?.select_one(&format!("#{id}"))?.placement())
}

#[cfg(feature = "core-input")]
fn input(html: &str) -> Result<String, Box<dyn std::error::Error>> {
    // Use the production MIME/sanitizer/cache path, not a copied sanitizer.
    let temp = tempfile::tempdir()?;
    let engine =
        lightmail_core::MailEngine::new(temp.path().join("Mail").to_string_lossy().into())?;
    engine.seed_demo()?;
    let source = temp.path().join("synthetic.eml");
    std::fs::write(&source, format!("From: synthetic@example.invalid\r\nTo: demo-work@example.com\r\nSubject: HTML reader probe\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{html}"))?;
    let m = engine.import_eml(source.to_string_lossy().into(), "demo-work".into())?;
    Ok(engine
        .cached_body(m.id)?
        .ok_or("Imported body missing")?
        .html)
}
#[cfg(not(feature = "core-input"))]
fn input(html: &str) -> Result<String, Box<dyn std::error::Error>> {
    Ok(html.into())
}

fn probe(
    name: &str,
    raw: &str,
    width: u32,
    scale: f32,
    output: &Path,
    private: bool,
) -> Result<Value, Box<dyn std::error::Error>> {
    let html = if private { raw.into() } else { input(raw)? };
    let started = Instant::now();
    let trace = Rc::new(RefCell::new(Trace::default()));
    let mut container = Container {
        pixels: PixbufContainer::new_with_scale(width, 640, scale),
        trace: trace.clone(),
    };
    let font_init_ms = started.elapsed().as_secs_f64() * 1000.;
    let measure = container.pixels.text_measure_fn();
    let started = Instant::now();
    let mut doc = Document::from_html(
        &html,
        &mut container,
        None,
        Some("body { background:white; }"),
    )?;
    let _ = doc.render(width as f32);
    let parse_layout_ms = started.elapsed().as_secs_f64() * 1000.;
    let height = doc.height();
    let content_width = doc.width();
    let mut geometry = BTreeMap::new();
    for id in [
        "brand",
        "first",
        "second",
        "receipt",
        "item",
        "amount",
        "spanrows",
        "rowone",
        "rowtwo",
        "spancols",
        "breaks",
        "wrap",
        "legacy",
        "legacy-cell",
        "select",
        "after",
        "action",
        "tail",
    ] {
        if let Some(p) = placement(&doc, id) {
            geometry.insert(id, pos(p));
        }
    }
    let started = Instant::now();
    doc.draw(
        DrawContext::default(),
        0.,
        0.,
        Some(Position {
            width: width as f32,
            height: 640.,
            ..Default::default()
        }),
    );
    let paint_ms = started.elapsed().as_secs_f64() * 1000.;
    let mut checks = BTreeMap::new();
    if name.starts_with("layout") {
        let p = |id| placement(&doc, id).unwrap_or_default();
        checks.insert(
            "nested_table_has_columns",
            p("amount").x > p("item").x + p("item").width * 0.5,
        );
        checks.insert(
            "cell_paragraphs_separate",
            p("second").y > p("first").y + 10.,
        );
        checks.insert(
            "colspan_covers_two_columns",
            p("spancols").x <= p("item").x + 2.
                && p("spancols").x + p("spancols").width >= p("amount").x + p("amount").width - 2.,
        );
        checks.insert(
            "rowspan_covers_two_rows",
            p("spanrows").height >= p("rowone").height + p("rowtwo").height - 2.,
        );
        checks.insert("br_keeps_three_lines", p("breaks").height > 45.);
        checks.insert(
            "responsive_card_fits_viewport",
            p("brand").width <= width as f32 + 1.,
        );
        checks.insert(
            "class_rule_sets_brand_color",
            trace
                .borrow()
                .solid
                .iter()
                .any(|v| v["color"] == json!([34, 100, 81, 255])),
        );
        checks.insert(
            "media_query_changes_font",
            doc.root()
                .and_then(|r| r.select_one("#brand"))
                .is_some_and(|e| (e.font_size() - if width <= 480 { 20. } else { 16. }).abs() < 1.),
        );
    }
    if name == "long-url" {
        checks.insert(
            "long_unbroken_link_fits_viewport",
            content_width <= width as f32 + 1.,
        );
    }
    if name.starts_with("legacy") {
        let cell = placement(&doc, "legacy-cell").unwrap_or_default();
        checks.insert(
            "legacy_table_width",
            placement(&doc, "legacy").is_some_and(|p| (p.width - 600.).abs() < 3.),
        );
        checks.insert(
            "legacy_cellpadding",
            placement(&doc, "legacy").is_some_and(|p| p.height >= cell.height + 46.),
        );
    }
    let mut selected = None;
    if name == "selection" {
        let p = placement(&doc, "select").ok_or("Selection paragraph missing")?;
        let mut sel = Selection::for_document(&doc);
        sel.start_at(
            &doc,
            &measure,
            p.x + 0.1,
            p.y + p.height / 2.,
            p.x + 0.1,
            p.y + p.height / 2.,
        );
        sel.extend_to(
            &doc,
            &measure,
            p.x + p.width - 0.1,
            p.y + p.height / 2.,
            p.x + p.width - 0.1,
            p.y + p.height / 2.,
        );
        let text = sel.selected_text().unwrap_or_default();
        checks.insert("chinese_selection_exact", text.trim() == "Alpha 中文 Beta");
        checks.insert(
            "selection_highlight_rectangles",
            !sel.rectangles().is_empty(),
        );
        selected = Some(text);
    }
    if let Some(element) = doc.root().and_then(|r| r.select_one("#action")) {
        let p = element
            .inline_boxes()
            .first()
            .copied()
            .unwrap_or_else(|| element.placement());
        let x = p.x + p.width / 2.;
        let y = p.y + p.height / 2.;
        doc.on_mouse_over(x, y, x, y);
        doc.on_lbutton_down(x, y, x, y);
        doc.on_lbutton_up(x, y, x, y);
        checks.insert(
            "signed_anchor_exact",
            trace.borrow().clicked.as_deref()
                == Some("https://example.invalid/verify?t=a%2Bb%3D&next=%2Finbox"),
        );
    }
    let mut tail_draw_ms = None;
    if name == "long" {
        // Fixed-size viewport even when the document is thousands of pixels tall.
        trace.borrow_mut().text.clear();
        let started = Instant::now();
        doc.draw(
            DrawContext::default(),
            0.,
            -(height - 640.).max(0.),
            Some(Position {
                width: width as f32,
                height: 640.,
                ..Default::default()
            }),
        );
        tail_draw_ms = Some(started.elapsed().as_secs_f64() * 1000.);
        checks.insert(
            "long_tail_is_painted",
            trace.borrow().text.iter().any(|v| {
                v["text"]
                    .as_str()
                    .is_some_and(|s| s.contains("TAIL_MARKER"))
            }),
        );
    }
    let measured_before = trace.borrow().measured;
    let started = Instant::now();
    let _ = doc.render(width as f32);
    let relayout_ms = started.elapsed().as_secs_f64() * 1000.;
    drop(doc);
    let t = trace.borrow();
    let filename = format!("{name}-{width}-{}", (scale * 100.).round() as u32);
    image::save_buffer(
        output.join(format!("{filename}.png")),
        container.pixels.pixels(),
        container.pixels.width(),
        container.pixels.height(),
        image::ColorType::Rgba8,
    )?;
    Ok(
        json!({"fixture":name,"width":width,"scale":scale,"inputBytes":html.len(),"documentWidth":content_width,"documentHeight":height,
        "surfaceBytes":container.pixels.pixels().len(),"fontInitMs":font_init_ms,"parseLayoutMs":parse_layout_ms,"paintMs":paint_ms,"relayoutMs":relayout_ms,"tailDrawMs":tail_draw_ms,
        "fonts":t.fonts,"measureCalls":measured_before,"relayoutMeasureCalls":t.measured-measured_before,
        "checks":checks,"geometry":if private { Value::Null } else { json!(geometry) },
        "selection":if private { None } else { selected },"anchor":if private { None } else { t.clicked.as_ref() },
        "imageCallbackCount":t.image_callbacks.len(),"cssCallbackCount":t.css_callbacks.len(),
        "imageCallbacks":if private { Value::Null } else { json!(t.image_callbacks) },"cssCallbacks":if private { Value::Null } else { json!(t.css_callbacks) },
        "textDraws":if private { Value::Null } else { json!(t.text) },"textDrawCount":t.text.len(),
        "fills":if private { Value::Null } else { json!(t.solid) },"networkRequests":0,"memory":memory(),
        "viewportOverflow":content_width > width as f32 + 1.}),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "--private") {
        let source = Path::new(args.get(1).ok_or("Pass private sample directory")?);
        let output = Path::new(args.get(2).ok_or("Pass private output directory")?);
        // Prevent accidentally publishing or mixing live mail with synthetic data.
        if !output.to_string_lossy().contains("private") {
            return Err("Use a private output directory".into());
        }
        std::fs::create_dir_all(output)?;
        let mut files = std::fs::read_dir(source)?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "html"))
            .collect::<Vec<_>>();
        files.sort();
        let mut results = vec![];
        for (index, file) in files.iter().enumerate() {
            let raw = std::fs::read_to_string(file)?;
            let name = format!("real-{:02}", index + 1);
            for (width, scale) in [(360, 1.), (600, 1.), (720, 1.), (720, 1.5), (720, 2.)] {
                match probe(&name,&raw,width,scale,output,true) {
                    Ok(value) => results.push(value),
                    Err(error) => results.push(json!({"fixture":name,"width":width,"scale":scale,"error":error.to_string()})),
                }
            }
            println!("Checked private sample {}/{}", index + 1, files.len());
        }
        std::fs::write(
            output.join("report.json"),
            serde_json::to_vec_pretty(
                &json!({"input":"already-cached private HTML","renderer":"litehtml CPU Pixbuf, not GPUI","results":results}),
            )?,
        )?;
        println!("Private report contains metrics only; PNGs contain real mail and stay local.");
        if results.iter().any(|v| v.get("error").is_some()) {
            return Err(
                "Some private inputs could not be rendered; inspect local metrics report".into(),
            );
        }
        return Ok(());
    }
    let directory = args
        .first()
        .ok_or("Usage: lightmail-html-reader-probe OUTPUT")?;
    let output = Path::new(&directory);
    std::fs::create_dir_all(output)?;
    let long = format!("<html><body style='margin:0;font:16px sans-serif'>{}<p id='tail'>TAIL_MARKER</p></body></html>", (0..400).map(|i|format!("<p>Long synthetic row {i}: 中文与 English.</p>")).collect::<String>());
    let responsive = include_str!("../fixtures/layout.html").replace(
        "@media (max-width:480px) {",
        "@media (max-width:480px) { .card { width:100%; }",
    );
    let legacy_inline = include_str!("../fixtures/legacy.html").replace(
        "id=\"legacy-cell\"",
        "id=\"legacy-cell\" style=\"padding:24px\"",
    );
    let fixtures = [
        ("layout", include_str!("../fixtures/layout.html")),
        ("layout-responsive", responsive.as_str()),
        ("selection", include_str!("../fixtures/selection.html")),
        ("resources", include_str!("../fixtures/resources.html")),
        ("legacy", include_str!("../fixtures/legacy.html")),
        ("legacy-inline", legacy_inline.as_str()),
        ("long-url", include_str!("../fixtures/long-url.html")),
        ("long", long.as_str()),
    ];
    let mut results = vec![];
    for (name, source) in fixtures {
        for (width, scale) in [(360, 1.), (720, 1.), (720, 1.5), (720, 2.)] {
            match probe(name, source, width, scale, output, false) {
                Ok(value) => {
                    println!(
                        "{name}: {width}px scale {scale}, layout {:.1}ms",
                        value["parseLayoutMs"].as_f64().unwrap_or_default()
                    );
                    results.push(value);
                }
                Err(error) => results.push(
                    json!({"fixture":name,"width":width,"scale":scale,"error":error.to_string()}),
                ),
            }
        }
    }
    let report = json!({"bindingRevision":"662ed3a6cbad72be8b9d76a8fc01e13faa8258e8","engineRevision":"8836bc1bc35ca0cfd71dc0386ef841d5cbc3bd5e",
        "probeExeBytes":std::fs::metadata(std::env::current_exe()?)?.len(),
        "input":if cfg!(feature="core-input") {"production MIME sanitizer"} else {"raw synthetic HTML"},
        "renderer":"CPU Pixbuf probe, not GPUI", "results":results});
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!("Saved {} cases to {}", results.len(), output.display());
    if results.iter().any(|v| v.get("error").is_some()) {
        return Err("Some synthetic inputs could not be rendered; inspect report".into());
    }
    Ok(())
}
