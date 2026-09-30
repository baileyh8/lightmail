use anyrender::ImageRenderer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{BaseDocument, DocumentConfig, NodeId, StyleThreading};
use blitz_html::HtmlDocument;
use blitz_traits::{
    net::DummyNetProvider,
    shell::{ColorScheme, Viewport},
};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Instant};

fn layout(doc: &BaseDocument, id: &str) -> Option<Value> {
    let node = doc.get_element_by_id(id).and_then(|id| doc.get_node(id))?;
    let layout = node.final_layout();
    let overflow_width =
        layout.scrollable_overflow_rect.right - layout.scrollable_overflow_rect.left;
    let overflow_height =
        layout.scrollable_overflow_rect.bottom - layout.scrollable_overflow_rect.top;
    Some(json!({
        "x": layout.location.x, "y": layout.location.y,
        "width": layout.size.width, "height": layout.size.height,
        "scrollWidth": overflow_width,
        "scrollHeight": overflow_height,
        "text": node.text_content()
    }))
}

fn node_id(doc: &BaseDocument, id: &str) -> Option<NodeId> {
    doc.get_element_by_id(id)
}

fn probe(
    name: &str,
    html: &str,
    width: u32,
    scale: f32,
) -> Result<Value, Box<dyn std::error::Error>> {
    let started = Instant::now();
    let config = DocumentConfig {
        viewport: Some(Viewport::new(width, 640, scale, ColorScheme::Light)),
        net_provider: Some(Arc::new(DummyNetProvider)),
        style_threading: StyleThreading::Sequential,
        ..Default::default()
    };
    let mut doc = HtmlDocument::from_html(html, config);
    doc.resolve(0.0);
    let resolve_ms = started.elapsed().as_secs_f64() * 1000.;
    let root = doc.root_element();
    let root_layout = root.final_layout();
    let mut checks = serde_json::Map::new();
    if name.starts_with("layout") {
        let item = layout(&doc, "item").unwrap_or_default();
        let amount = layout(&doc, "amount").unwrap_or_default();
        let span_rows = layout(&doc, "spanrows").unwrap_or_default();
        let row_one = layout(&doc, "rowone").unwrap_or_default();
        let row_two = layout(&doc, "rowtwo").unwrap_or_default();
        let span_cols = layout(&doc, "spancols").unwrap_or_default();
        let brand = layout(&doc, "brand").unwrap_or_default();
        let first = layout(&doc, "first").unwrap_or_default();
        let second = layout(&doc, "second").unwrap_or_default();
        checks.insert(
            "nested_table_has_columns".into(),
            json!(amount["x"].as_f64().unwrap_or(0.) > item["x"].as_f64().unwrap_or(0.) + 10.),
        );
        checks.insert(
            "cell_paragraphs_separate".into(),
            json!(second["y"].as_f64().unwrap_or(0.) > first["y"].as_f64().unwrap_or(0.) + 10.),
        );
        checks.insert(
            "rowspan_covers_two_rows".into(),
            json!(
                span_rows["height"].as_f64().unwrap_or(0.)
                    >= row_one["height"].as_f64().unwrap_or(0.)
                        + row_two["height"].as_f64().unwrap_or(0.)
                        - 2.
            ),
        );
        checks.insert(
            "colspan_covers_two_columns".into(),
            json!(
                span_cols["width"].as_f64().unwrap_or(0.)
                    > item["width"].as_f64().unwrap_or(0.) + amount["width"].as_f64().unwrap_or(0.)
                        - 5.
            ),
        );
        checks.insert(
            "responsive_card_fits_viewport".into(),
            json!(brand["width"].as_f64().unwrap_or(99999.) <= width as f64 + 1.),
        );
        checks.insert(
            "paragraph_has_height".into(),
            json!(first["height"].as_f64().unwrap_or(0.) > 0.),
        );
    }
    if name == "selection" {
        let selected = layout(&doc, "select").unwrap_or_default();
        checks.insert(
            "chinese_text_present".into(),
            json!(
                selected["text"]
                    .as_str()
                    .is_some_and(|s| s.contains("中文"))
            ),
        );
        checks.insert(
            "point_hits_action".into(),
            json!(doc.element_from_point(10., 10.).is_some()),
        );
    }
    if name == "long-url" {
        checks.insert(
            "long_url_fits_viewport".into(),
            json!(
                (root_layout.scrollable_overflow_rect.right
                    - root_layout.scrollable_overflow_rect.left)
                    <= width as f32 + 1.
            ),
        );
    }
    let ids = [
        "brand", "first", "second", "receipt", "item", "amount", "spanrows", "rowone", "rowtwo",
        "spancols", "select", "after", "action", "wrap",
    ];
    let mut geometry = serde_json::Map::new();
    for id in ids {
        if let Some(value) = layout(&doc, id) {
            geometry.insert(id.into(), value);
        }
    }
    let action_point = node_id(&doc, "action")
        .and_then(|id| doc.get_node(id))
        .map(|node| {
            let l = node.final_layout();
            doc.element_from_point(l.location.x + 1., l.location.y + 1.)
                .is_some()
        });
    let root_overflow_width =
        root_layout.scrollable_overflow_rect.right - root_layout.scrollable_overflow_rect.left;
    let root_overflow_height =
        root_layout.scrollable_overflow_rect.bottom - root_layout.scrollable_overflow_rect.top;
    Ok(
        json!({"fixture":name,"width":width,"scale":scale,"resolveMs":resolve_ms,
        "rootWidth":root_layout.size.width,"rootHeight":root_layout.size.height,
        "scrollWidth":root_overflow_width,"scrollHeight":root_overflow_height,
        "viewportOverflow":root_overflow_width > width as f32 + 1.,
        "checks":checks,"geometry":geometry,"actionHit":action_point,"pendingCriticalResources":doc.has_pending_critical_resources()}),
    )
}

fn render_png(
    name: &str,
    html: &str,
    width: u32,
    height: u32,
    output: &Path,
) -> Result<Value, Box<dyn std::error::Error>> {
    let config = DocumentConfig {
        viewport: Some(Viewport::new(width, height, 1.0, ColorScheme::Light)),
        net_provider: Some(Arc::new(DummyNetProvider)),
        style_threading: StyleThreading::Sequential,
        ..Default::default()
    };
    let mut doc = HtmlDocument::from_html(html, config);
    doc.resolve(0.0);
    let root = doc.root_element().final_layout();
    let document_height = root.size.height.max(root.scrollable_overflow_rect.bottom);
    if !document_height.is_finite() || document_height > 16_000. {
        return Err("Document exceeds the bounded screenshot height".into());
    }
    let capture_height = (document_height.ceil() as u32 + 1).max(height);
    let mut renderer = VelloCpuImageRenderer::new(width, capture_height);
    let mut pixels = Vec::new();
    renderer.render_to_vec(
        |scene| blitz_paint::paint_scene(scene, &mut doc, 1.0, width, capture_height, 0, 0),
        &mut pixels,
    );
    // Vello emits premultiplied RGBA. Composite onto the reader's white canvas
    // before encoding PNG; otherwise transparent page margins look black.
    for pixel in pixels.chunks_exact_mut(4) {
        let background = 255u16 - pixel[3] as u16;
        for channel in &mut pixel[..3] {
            *channel = (*channel as u16 + background).min(255) as u8;
        }
        pixel[3] = 255;
    }
    let path = output.join(format!("{name}-{width}.png"));
    image::save_buffer(
        &path,
        &pixels[..(width * height * 4) as usize],
        width,
        height,
        image::ColorType::Rgba8,
    )?;
    let full_path = output.join(format!("{name}-{width}-full.png"));
    image::save_buffer(
        &full_path,
        &pixels,
        width,
        capture_height,
        image::ColorType::Rgba8,
    )?;
    Ok(
        json!({"sample":name,"width":width,"height":height,"path":path.to_string_lossy(),
            "fullPath":full_path.to_string_lossy(),"captureHeight":capture_height,
            "rootHeight":doc.root_element().final_layout().size.height,
            "renderer":"Blitz Paint / AnyRender Vello CPU", "remoteImages":false}),
    )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (output, sources): (&Path, Vec<(String, String)>);
    let mut owned = Vec::new();
    let render = args.first().is_some_and(|v| v == "--private-render");
    if args
        .first()
        .is_some_and(|v| v == "--private" || v == "--private-render")
    {
        output = Path::new(args.get(2).ok_or("Pass private input and output")?);
        let directory = Path::new(args.get(1).unwrap());
        for path in std::fs::read_dir(directory)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "html"))
        {
            owned.push((
                path.file_stem().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(path)?,
            ));
        }
        owned.sort_by(|a, b| a.0.cmp(&b.0));
        sources = owned.clone();
    } else {
        output = Path::new(args.first().ok_or("Pass output directory")?);
        sources = vec![
            (
                "layout".into(),
                include_str!("../../html-reader-probe/fixtures/layout.html").into(),
            ),
            (
                "layout-responsive".into(),
                include_str!("../../html-reader-probe/fixtures/layout.html").replace(
                    "@media (max-width:480px) {",
                    "@media (max-width:480px) { .card { width:100%; }",
                ),
            ),
            (
                "selection".into(),
                include_str!("../../html-reader-probe/fixtures/selection.html").into(),
            ),
            (
                "long-url".into(),
                include_str!("../../html-reader-probe/fixtures/long-url.html").into(),
            ),
        ];
    }
    std::fs::create_dir_all(output)?;
    if render {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        if !output
            .canonicalize()?
            .starts_with(workspace.join("build").canonicalize()?)
        {
            return Err("Private screenshots must stay under the ignored build directory".into());
        }
        if sources.len() < 20 {
            return Err("Need at least 20 cached HTML samples".into());
        }
        let mut screenshots = Vec::new();
        for (index, (name, html)) in sources.iter().take(20).enumerate() {
            screenshots.push(render_png(
                &format!("{:02}-{}", index + 1, name),
                html,
                720,
                640,
                output,
            )?);
        }
        std::fs::write(
            output.join("render-manifest.json"),
            serde_json::to_vec_pretty(&screenshots)?,
        )?;
        println!("Blitz rendered {} private samples", screenshots.len());
        return Ok(());
    }
    let mut results = Vec::new();
    for (name, html) in sources {
        for (width, scale) in [(360, 1.), (600, 1.), (720, 1.), (720, 1.5), (720, 2.)] {
            results.push(probe(&name, &html, width, scale)?);
        }
    }
    std::fs::write(
        output.join("report.json"),
        serde_json::to_vec_pretty(
            &json!({"engine":"Blitz DOM: Stylo + Taffy + Parley","results":results}),
        )?,
    )?;
    println!("Blitz: {} cases completed", results.len());
    Ok(())
}
