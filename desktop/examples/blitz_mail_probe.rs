//! Exercise the product reader worker with previously extracted local HTML.
//! Does not initialize mail services, credentials or network image permission.
#![allow(dead_code)]
#[path = "../src/blitz_reader.rs"]
mod blitz_reader;
#[path = "../src/image_types.rs"]
mod image_types;
#[cfg(test)]
#[path = "../src/platform.rs"]
mod platform;
use blitz_reader::{ReaderViewport, Select, Worker};
use lightmail_core::{PlatformServices, ProxyRoute, Result};
use serde_json::json;
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

struct Offline;
impl PlatformServices for Offline {
    fn read_secret(&self, _: String) -> Result<Option<String>> {
        Ok(None)
    }
    fn write_secret(&self, _: String, _: String) -> Result<()> {
        unreachable!("No credentials in reader probe")
    }
    fn remove_secret(&self, _: String) -> Result<()> {
        unreachable!("No credentials in reader probe")
    }
    fn proxy_for(&self, _: String) -> Result<ProxyRoute> {
        unreachable!("External images disabled")
    }
}
fn receive<T>(rx: async_channel::Receiver<T>) -> anyhow::Result<T> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match rx.try_recv() {
            Ok(value) => return Ok(value),
            Err(async_channel::TryRecvError::Closed) => anyhow::bail!("Reader request cancelled"),
            Err(_) => {
                anyhow::ensure!(Instant::now() < deadline, "Reader request timed out");
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}
fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        args.len() == 2 || args.len() == 3,
        "Usage: blitz_mail_probe PRIVATE_HTML BUILD_OUTPUT [SAMPLE_INDEX]"
    );
    let output = Path::new(&args[1]);
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let build = workspace.join("build").canonicalize()?;
    std::fs::create_dir_all(output)?;
    anyhow::ensure!(
        output.canonicalize()?.starts_with(build),
        "Private reader output must stay in build/"
    );
    let mut files = std::fs::read_dir(&args[0])?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "html"))
        .collect::<Vec<_>>();
    files.sort();
    anyhow::ensure!(files.len() >= 20, "Need twenty cached HTML files");
    let worker = Worker::new();
    let mut generation = 0;
    let mut metrics = Vec::new();
    let sample = args.get(2).map(|s| s.parse::<usize>()).transpose()?;
    anyhow::ensure!(
        sample.is_none_or(|sample| (1..=20).contains(&sample)),
        "Sample index must be 1..20"
    );
    for (index, file) in files.iter().take(20).enumerate() {
        if sample.is_some_and(|sample| sample != index + 1) {
            continue;
        }
        let html = std::fs::read_to_string(file)?;
        for width in [360, 600, 720] {
            for scale in [1., 1.5, 2.] {
                generation += 1;
                let started = Instant::now();
                let rendered = receive(worker.load(
                    generation,
                    index.to_string(),
                    html.clone(),
                    ReaderViewport {
                        width,
                        height: 640,
                        scale,
                    },
                    false,
                    Arc::new(Offline),
                ))??;
                let render_ms = started.elapsed().as_secs_f64() * 1000.;
                let selected = receive(worker.select(generation, Select::All))?;
                anyhow::ensure!(
                    !selected.text.trim().is_empty() && !selected.rects.is_empty(),
                    "Cached sample {} at width {width}, scale {scale} produced no selectable text",
                    index + 1
                );
                // Decode like GPUI to verify the actual PNG, including physical DPI.
                let pixels = image::load_from_memory_with_format(
                    &rendered.image.bytes,
                    image::ImageFormat::Png,
                )?;
                anyhow::ensure!(
                    pixels.width() >= (width as f32 * scale) as u32,
                    "Physical viewport too narrow"
                );
                if width == 720 && scale == 1. {
                    std::fs::write(
                        output.join(format!("private-mail-{:02}.png", index + 1)),
                        &rendered.image.bytes,
                    )?;
                }
                metrics.push(json!({"sample":index+1,"width":width,"scale":scale,"renderMs":render_ms,"logicalHeight":rendered.height,"physicalPixels":[pixels.width(),pixels.height()],"selectionBytes":selected.text.len(),"selectionRects":selected.rects.len(),"horizontalOverflow":rendered.width>width as f32+1.}));
            }
        }
    }
    worker.clear(generation + 1);
    std::fs::write(
        output.join("metrics.json"),
        serde_json::to_vec_pretty(
            &json!({"samples":if sample.is_some(){1}else{20},"checks":metrics,"imagesAllowed":false,"reader":"production Worker + Session; actual PNG decoded; no mail services"}),
        )?,
    )?;
    println!(
        "{} cached samples: {} production reader render/decode/selection checks completed",
        if sample.is_some() { 1 } else { 20 },
        metrics.len()
    );
    Ok(())
}
