//! Blitz HTML/CSS reader adapter.
//!
//! The DOM/layout engine runs off the GPUI view path and produces a bounded PNG
//! surface for the first integration step. GPUI remains the window and scroll
//! host; no Blitz shell or WebView is involved. Resource requests use the same
//! core downloader as the existing native image reader.
use anyrender::ImageRenderer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{DocumentConfig, StyleThreading};
use blitz_html::HtmlDocument;
use blitz_traits::{
    net::{Bytes, NetHandler, NetProvider, Request},
    shell::{ColorScheme, Viewport},
};
use gpui_kit::{Image, ImageFormat};
use lightmail_core::PlatformServices;
use std::{collections::HashSet, sync::Arc};

const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_DOCUMENT_HEIGHT: f32 = 16_000.;

pub struct Rendered {
    pub image: Arc<Image>,
    pub width: f32,
    pub height: f32,
    #[allow(dead_code)]
    pub source: String,
    pub links: Vec<LinkHit>,
}

#[derive(Clone)]
pub struct LinkHit {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub href: String,
}

struct MailNet {
    platform: Arc<dyn PlatformServices>,
    allow_images: bool,
    allowed_images: HashSet<String>,
}

impl NetProvider for MailNet {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.to_string();
        if !self.allow_images || (!url.starts_with("data:") && !self.allowed_images.contains(&url))
        {
            handler.bytes(url, Bytes::new());
            return;
        }

        // Keep the first adapter synchronous. `render` is called from the
        // background reader task, so the GPUI thread never waits on a network
        // response. A later version can move this into an image job queue.
        let bytes = if let Some((mime, payload)) = url.strip_prefix("data:").and_then(data_url) {
            decode_data_url(mime, payload)
        } else if matches!(request.url.scheme(), "http" | "https") {
            lightmail_core::fetch_resource(self.platform.clone(), url.clone(), MAX_IMAGE_BYTES)
                .ok()
                .map(|(bytes, _mime)| bytes)
        } else {
            None
        };
        handler.bytes(url, Bytes::from(bytes.unwrap_or_default()));
    }

    fn is_noop(&self) -> bool {
        !self.allow_images
    }
}

fn data_url(value: &str) -> Option<(&str, &str)> {
    let (header, payload) = value.split_once(',')?;
    let mime = header.strip_suffix(";base64")?;
    Some((mime, payload))
}

fn decode_data_url(mime: &str, payload: &str) -> Option<Vec<u8>> {
    if !mime.starts_with("image/") {
        return None;
    }
    let bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, payload.trim()).ok()?;
    (bytes.len() <= MAX_IMAGE_BYTES).then_some(bytes)
}

fn image_sources(html: &str) -> HashSet<String> {
    let mut sources = HashSet::new();
    let mut cursor = html;
    while let Some(start) = cursor.to_ascii_lowercase().find("<img") {
        cursor = &cursor[start..];
        let Some(end) = cursor.find('>') else { break };
        let tag = &cursor[..end];
        let lower = tag.to_ascii_lowercase();
        if let Some(src) = lower.find("src=") {
            let value = &tag[src + 4..];
            if let Some(quote) = value.as_bytes().first().copied().map(char::from) {
                if matches!(quote, '\'' | '"') {
                    if let Some(end_quote) = value[1..].find(quote) {
                        sources.insert(value[1..1 + end_quote].to_string());
                    }
                }
            }
        }
        cursor = &cursor[end + 1..];
    }
    sources
}

fn encode_png(mut pixels: Vec<u8>, width: u32, height: u32) -> anyhow::Result<Vec<u8>> {
    // Vello CPU returns premultiplied RGBA. Email readers display against a
    // white page, so flatten alpha before handing bytes to GPUI's decoder.
    for pixel in pixels.chunks_exact_mut(4) {
        let background = 255u16 - pixel[3] as u16;
        for channel in &mut pixel[..3] {
            *channel = (*channel as u16 + background).min(255) as u8;
        }
        pixel[3] = 255;
    }
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&pixels)?;
    Ok(output)
}

/// Render sanitized HTML with Blitz and return a GPUI image surface.
pub fn render(
    html: &str,
    width: u32,
    allow_images: bool,
    platform: Arc<dyn PlatformServices>,
) -> anyhow::Result<Rendered> {
    let config = DocumentConfig {
        viewport: Some(Viewport::new(width, 640, 1., ColorScheme::Light)),
        net_provider: Some(Arc::new(MailNet {
            platform,
            allow_images,
            allowed_images: image_sources(html),
        })),
        style_threading: StyleThreading::Sequential,
        ..Default::default()
    };
    let mut document = HtmlDocument::from_html(html, config);
    // The first pass discovers resources. A second pass consumes synchronous
    // responses and incorporates image dimensions into layout.
    for _ in 0..3 {
        document.resolve(0.);
    }
    let root = document.root_element().final_layout();
    let height = root.size.height.max(root.scrollable_overflow_rect.bottom);
    if !height.is_finite() || height > MAX_DOCUMENT_HEIGHT {
        anyhow::bail!("邮件正文超过原生 HTML 阅读上限");
    }
    let height = height.ceil().max(640.) as u32;
    let links = document
        .query_selector_all("a")
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|id| {
            let href = document
                .get_node(id)?
                .attr(blitz_dom::local_name!("href"))?
                .to_string();
            let rect = document.get_client_bounding_rect(id)?;
            (rect.width > 0. && rect.height > 0.).then_some(LinkHit {
                x: rect.x as f32,
                y: rect.y as f32,
                width: rect.width as f32,
                height: rect.height as f32,
                href,
            })
        })
        .collect();
    let mut renderer = VelloCpuImageRenderer::new(width, height);
    let mut pixels = Vec::new();
    renderer.render_to_vec(
        |scene| blitz_paint::paint_scene(scene, &mut document, 1., width, height, 0, 0),
        &mut pixels,
    );
    let png = encode_png(pixels, width, height)?;
    Ok(Rendered {
        image: Arc::new(Image::from_bytes(ImageFormat::Png, png)),
        width: width as f32,
        height: height as f32,
        source: html.to_string(),
        links,
    })
}

#[cfg(test)]
mod tests {
    use super::{image_sources, render};
    use crate::platform::DesktopPlatform;
    use std::sync::Arc;
    use std::{
        io::{Read, Write},
        time::Duration,
    };

    #[test]
    fn embedded_images_follow_the_message_permission() {
        let html = "<html><body><img src='data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=='></body></html>";
        let platform = Arc::new(DesktopPlatform::default());
        let blocked = render(html, 720, false, platform.clone()).unwrap();
        let allowed = render(html, 720, true, platform).unwrap();
        assert!(allowed.height >= blocked.height);
        assert_eq!(allowed.width, 720.);
    }

    #[test]
    fn remote_images_use_the_core_downloader_only_after_opt_in() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let pixel = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==",
        )
        .unwrap();
        let response = pixel.clone();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = (0..100)
                .find_map(|_| match listener.accept() {
                    Ok(connection) => Some(connection),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10));
                        None
                    }
                    Err(error) => panic!("image fixture accept failed: {error}"),
                })
                .expect("image fixture request did not arrive");
            let mut request = [0u8; 2048];
            let _ = socket.read(&mut request);
            write!(
                socket,
                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response.len()
            )
            .unwrap();
            socket.write_all(&response).unwrap();
        });
        let html = format!("<html><body><img src='http://{address}/pixel.png'></body></html>");
        assert!(image_sources(&html).contains(&format!("http://{address}/pixel.png")));
        let platform = Arc::new(DesktopPlatform::default());
        let blocked = render(&html, 720, false, platform.clone()).unwrap();
        let allowed = render(&html, 720, true, platform).unwrap();
        server.join().unwrap();
        assert!(allowed.height >= blocked.height);
    }
}
