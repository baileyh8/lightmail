//! Blitz HTML/CSS reader adapter.
//!
//! The DOM/layout engine runs off the GPUI view path and rasterizes a bounded
//! visible region with overscan. GPUI remains the window and scroll
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
use parley::{Affinity, Cursor, Selection as ParleySelection};
use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

#[path = "blitz_reader/worker.rs"]
mod worker;
pub use worker::Worker;
#[path = "blitz_reader/downloads.rs"]
mod downloads;

const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const MAX_DOCUMENT_HEIGHT: f32 = 2_000_000.;
const MAX_SURFACE_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_MESSAGE_BYTES: usize = 32 * 1024 * 1024;
const MAX_MESSAGE_IMAGES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReaderViewport {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}
impl Default for ReaderViewport {
    fn default() -> Self {
        Self {
            width: 720,
            height: 640,
            scale: 1.,
        }
    }
}
impl ReaderViewport {
    pub fn new(width: f32, height: f32, scale: f32) -> Option<Self> {
        if !width.is_finite()
            || !height.is_finite()
            || !scale.is_finite()
            || width < 80.
            || height < 40.
            || !(0.5..=4.).contains(&scale)
        {
            return None;
        }
        Some(Self {
            width: width.round().min(4096.) as u32,
            height: height.round().min(4096.) as u32,
            scale,
        })
    }
    fn physical_size(self) -> (u32, u32) {
        (
            (self.width as f32 * self.scale).round() as u32,
            (self.height as f32 * self.scale).round() as u32,
        )
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}
#[derive(Clone, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
impl Rect {
    pub fn covers(&self, point: Point, viewport: ReaderViewport) -> bool {
        point.x >= self.x - 1.
            && point.y >= self.y - 1.
            && point.x + viewport.width as f32 <= self.x + self.width + 1.
            && point.y + viewport.height as f32 <= self.y + self.height + 1.
    }
}
#[derive(Clone, Default)]
pub struct Selected {
    pub text: String,
    pub rects: Vec<Rect>,
    pub accessible: Option<(TextPosition, TextPosition)>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextPosition {
    pub key: u64,
    pub character: usize,
}
pub enum Select {
    Clear,
    Range(Point, Point),
    All,
    Accessible(TextPosition, TextPosition),
}

pub struct Rendered {
    pub image: Arc<Image>,
    pub width: f32,
    pub height: f32,
    #[allow(dead_code)]
    pub source: Arc<str>,
    pub links: Arc<[LinkHit]>,
    pub viewport: ReaderViewport,
    pub selection: Option<Selected>,
    pub area: Rect,
    pub layout_revision: u64,
    pub accessible: Arc<[AccessibleText]>,
}
pub struct AccessibleText {
    pub key: u64,
    pub text: Arc<str>,
    pub bounds: Rect,
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
    state: Arc<Mutex<NetState>>,
    async_images: Option<downloads::Images>,
}
#[derive(Default)]
struct NetState {
    bytes: usize,
    pixels: u64,
    images: HashMap<String, Bytes>,
}
impl NetState {
    fn finish(&mut self, url: String, image: Option<(Vec<u8>, u64)>) -> Bytes {
        let bytes = match image {
            Some((bytes, pixels))
                if self.bytes + bytes.len() <= MAX_MESSAGE_BYTES
                    && self.pixels + pixels <= MAX_SURFACE_PIXELS =>
            {
                self.bytes += bytes.len();
                self.pixels += pixels;
                Bytes::from(bytes)
            }
            _ => Bytes::new(),
        };
        self.images.insert(url, bytes.clone());
        bytes
    }
}

impl NetProvider for MailNet {
    fn fetch(&self, _doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        let url = request.url.to_string();
        if !self.allow_images || !self.allowed_images.contains(&url) {
            handler.bytes(url, Bytes::new());
            return;
        }
        if let Some(images) = &self.async_images {
            if matches!(request.url.scheme(), "http" | "https") {
                images.fetch(url, handler, self.state.clone(), self.platform.clone());
                return;
            }
        }

        // Inline data is local; HTTP uses the separate downloader in production.
        let mut state = self.state.lock().unwrap();
        if let Some(bytes) = state.images.get(&url) {
            let bytes = bytes.clone();
            drop(state);
            handler.bytes(url, bytes);
            return;
        }
        if state.images.len() >= MAX_MESSAGE_IMAGES
            || state.bytes >= MAX_MESSAGE_BYTES
            || state.pixels >= MAX_SURFACE_PIXELS
        {
            handler.bytes(url, Bytes::new());
            return;
        }
        let limit = MAX_IMAGE_BYTES.min(MAX_MESSAGE_BYTES - state.bytes);
        let bytes = if let Some((mime, payload)) = url.strip_prefix("data:").and_then(data_url) {
            decode_data_url(mime, payload)
                .and_then(|bytes| image_pixels(mime, &bytes).map(|pixels| (bytes, pixels)))
        } else if matches!(request.url.scheme(), "http" | "https") {
            lightmail_core::fetch_resource(self.platform.clone(), url.clone(), limit)
                .ok()
                .and_then(|(bytes, mime)| image_pixels(&mime, &bytes).map(|pixels| (bytes, pixels)))
        } else {
            None
        };
        let bytes = state.finish(url.clone(), bytes.filter(|(bytes, _)| bytes.len() <= limit));
        drop(state);
        handler.bytes(url, bytes);
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
    if !mime.starts_with("image/") || payload.len() > MAX_IMAGE_BYTES / 3 * 4 + 4 {
        return None;
    }
    let bytes =
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, payload.trim()).ok()?;
    (bytes.len() <= MAX_IMAGE_BYTES).then_some(bytes)
}
#[cfg(test)]
fn valid_image(mime: &str, bytes: &[u8]) -> bool {
    image_pixels(mime, bytes).is_some()
}
fn image_pixels(mime: &str, bytes: &[u8]) -> Option<u64> {
    crate::image_types::pixels(mime, bytes)
}

fn image_sources(html: &str) -> HashSet<String> {
    // Use the same HTML parser as the layout engine, including whitespace,
    // attribute case and entity decoding. This parse has no network provider.
    let document = HtmlDocument::from_html(html, DocumentConfig::default());
    document
        .query_selector_all("img")
        .unwrap_or_default()
        .into_iter()
        .filter_map(|id| document.get_node(id)?.attr(blitz_dom::local_name!("src")))
        .filter_map(|src| url::Url::parse(src).ok())
        .map(|url| url.to_string())
        .collect()
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
#[cfg(test)]
pub fn render(
    html: &str,
    width: u32,
    allow_images: bool,
    platform: Arc<dyn PlatformServices>,
) -> anyhow::Result<Rendered> {
    let mut session = Session::new(
        html.to_string(),
        ReaderViewport {
            width,
            ..Default::default()
        },
        allow_images,
        platform,
    );
    session.paint()
}

/// Created, laid out and selected only on the reader thread. The DOM never
/// crosses threads; GPUI receives owned pixels and selection geometry.
struct Session {
    document: HtmlDocument,
    source: Arc<str>,
    viewport: ReaderViewport,
    cancelled: Arc<AtomicBool>,
    roots: Vec<blitz_dom::NodeId>,
    selected_range: SelectedRange,
    size: (f32, f32),
    links: Arc<[LinkHit]>,
    scroll: Point,
    layout_revision: u64,
    fixed_elements: bool,
    accessible: Arc<[AccessibleText]>,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
impl Session {
    #[cfg(test)]
    fn new(
        html: String,
        viewport: ReaderViewport,
        allow_images: bool,
        platform: Arc<dyn PlatformServices>,
    ) -> Self {
        Self::with_images(html, viewport, allow_images, platform, None)
    }
    fn with_images(
        html: String,
        viewport: ReaderViewport,
        allow_images: bool,
        platform: Arc<dyn PlatformServices>,
        async_images: Option<downloads::Images>,
    ) -> Self {
        let cancelled = async_images
            .as_ref()
            .map(|images| images.cancelled.clone())
            .unwrap_or_else(|| Arc::new(AtomicBool::new(false)));
        let (physical_width, physical_height) = viewport.physical_size();
        let config = DocumentConfig {
            viewport: Some(Viewport::new(
                physical_width,
                physical_height,
                viewport.scale,
                ColorScheme::Light,
            )),
            net_provider: Some(Arc::new(MailNet {
                platform,
                allow_images,
                allowed_images: if allow_images {
                    image_sources(&html)
                } else {
                    HashSet::new()
                },
                state: Arc::new(Mutex::new(NetState::default())),
                async_images,
            })),
            style_threading: StyleThreading::Sequential,
            ..Default::default()
        };
        let document = HtmlDocument::from_html(&html, config);
        Self {
            document,
            source: html.into(),
            viewport,
            cancelled,
            roots: Vec::new(),
            selected_range: SelectedRange::None,
            size: (0., 0.),
            links: Arc::from([]),
            scroll: Point::default(),
            layout_revision: 0,
            fixed_elements: false,
            accessible: Arc::from([]),
        }
    }
    fn resize(&mut self, viewport: ReaderViewport) {
        self.viewport = viewport;
        let (w, h) = viewport.physical_size();
        self.document
            .set_viewport(Viewport::new(w, h, viewport.scale, ColorScheme::Light));
    }
    fn paint(&mut self) -> anyhow::Result<Rendered> {
        let document = &mut self.document;
        // The first pass discovers resources. A second pass consumes synchronous
        // responses and incorporates image dimensions into layout.
        for _ in 0..3 {
            document.resolve(0.);
        }
        self.roots = visible_text_roots(document);
        self.accessible = self
            .roots
            .iter()
            .filter_map(|&id| {
                let node = document.get_node(id)?;
                let data = node.element_data()?.inline_layout_data.as_ref()?;
                let endpoint = Endpoint::new(document, id, 0);
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                endpoint.anonymous.hash(&mut hasher);
                if endpoint.anonymous.is_none() {
                    id.hash(&mut hasher);
                }
                let position = node.absolute_position(0., 0.);
                let layout = node.final_layout();
                Some(AccessibleText {
                    key: hasher.finish(),
                    text: format!("{}\n", data.text).into(),
                    bounds: Rect {
                        x: position.x + layout.padding.left + layout.border.left,
                        y: position.y + layout.padding.top + layout.border.top,
                        width: data.layout.width() / data.layout.scale(),
                        height: data.layout.height() / data.layout.scale(),
                    },
                })
            })
            .collect::<Vec<_>>()
            .into();
        document.clear_text_selection(); // highlight is a GPUI overlay, not baked into the PNG
        let root = document.root_element().final_layout();
        let height = root.size.height.max(root.scrollable_overflow_rect.bottom);
        if !height.is_finite() || height > MAX_DOCUMENT_HEIGHT {
            anyhow::bail!("邮件正文超过原生 HTML 阅读上限");
        }
        let height = height.ceil().max(self.viewport.height as f32);
        let width = root
            .size
            .width
            .max(root.scrollable_overflow_rect.right)
            .ceil()
            .max(self.viewport.width as f32);
        if !width.is_finite() || width > 4096. {
            anyhow::bail!("邮件正文过宽");
        }
        let links = document
            .query_selector_all("a")
            .ok()
            .into_iter()
            .flatten()
            .flat_map(|id| {
                let href = document
                    .get_node(id)?
                    .attr(blitz_dom::local_name!("href"))?
                    .to_string();
                Some(
                    document
                        .node_client_rects(id)
                        .into_iter()
                        .filter(|r| r.width > 0. && r.height > 0.)
                        .map(move |r| LinkHit {
                            x: r.x as f32,
                            y: r.y as f32,
                            width: r.width as f32,
                            height: r.height as f32,
                            href: href.clone(),
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .flatten()
            .collect::<Vec<_>>();
        self.links = links.into();
        self.size = (width, height);
        self.layout_revision = self.layout_revision.wrapping_add(1);
        self.fixed_elements = document.tree().iter().any(|(id, node)| {
            node.primary_styles().is_some()
                && document.resolved_style_value(id, "position") == "fixed"
        });
        self.paint_region(true)
    }
    fn paint_region(&mut self, layout_changed: bool) -> anyhow::Result<Rendered> {
        let (width, height) = self.size;
        let viewport = self.viewport;
        let x = self
            .scroll
            .x
            .clamp(0., (width - viewport.width as f32).max(0.));
        let y = self
            .scroll
            .y
            .clamp(0., (height - viewport.height as f32).max(0.));
        let top = if self.fixed_elements {
            y
        } else {
            (y / 256.).floor() * 256.
        };
        let extra = if self.fixed_elements { 0. } else { 512. };
        let surface_height = (viewport.height as f32 + extra).min(height - top);
        let left = if self.fixed_elements { x } else { 0. };
        let surface_width = if self.fixed_elements {
            viewport.width as f32
        } else {
            width
        };
        let area = Rect {
            x: left,
            y: top,
            width: surface_width,
            height: surface_height,
        };
        let physical_width = (surface_width * viewport.scale).ceil() as u32;
        let mut physical_height = (surface_height * viewport.scale).ceil() as u32;
        // Drop overscan when needed, never crop the visible viewport to fit.
        let visible_height = ((y - top + viewport.height as f32) * viewport.scale).ceil() as u32;
        physical_height =
            physical_height.min((MAX_SURFACE_PIXELS / physical_width.max(1) as u64) as u32);
        if physical_width > 16000 || physical_height > 16000 || physical_height < visible_height {
            anyhow::bail!("阅读视口超过绘图上限");
        }
        let area = Rect {
            height: physical_height as f32 / viewport.scale,
            ..area
        };
        let document = &mut self.document;
        document.set_viewport_scroll(blitz_dom::Point {
            x: left as f64,
            y: top as f64,
        });
        let mut renderer = VelloCpuImageRenderer::new(physical_width, physical_height);
        let mut pixels = Vec::new();
        renderer.render_to_vec(
            |scene| {
                blitz_paint::paint_scene(
                    scene,
                    document,
                    self.viewport.scale as f64,
                    physical_width,
                    physical_height,
                    0,
                    0,
                )
            },
            &mut pixels,
        );
        document.set_viewport_scroll(blitz_dom::Point { x: 0., y: 0. });
        let png = encode_png(pixels, physical_width, physical_height)?;
        let selection = layout_changed.then(|| self.selected());
        Ok(Rendered {
            image: Arc::new(Image::from_bytes(ImageFormat::Png, png)),
            width,
            height,
            source: self.source.clone(),
            links: self.links.clone(),
            viewport: self.viewport,
            selection,
            area,
            layout_revision: self.layout_revision,
            accessible: self.accessible.clone(),
        })
    }
    fn select(&mut self, selection: Select) -> Selected {
        let doc = &self.document;
        self.selected_range = match selection {
            Select::Clear => SelectedRange::None,
            Select::All => SelectedRange::All,
            Select::Accessible(a, b) => {
                let convert = |position: TextPosition| {
                    let index = self
                        .accessible
                        .iter()
                        .position(|block| block.key == position.key)?;
                    let id = *self.roots.get(index)?;
                    let text = &doc
                        .get_node(id)?
                        .element_data()?
                        .inline_layout_data
                        .as_ref()?
                        .text;
                    if position.character > text.chars().count() + 1 {
                        return None;
                    }
                    let byte = text
                        .char_indices()
                        .nth(position.character)
                        .map(|(i, _)| i)
                        .unwrap_or(text.len());
                    Some(Endpoint::new(doc, id, byte))
                };
                match (convert(a), convert(b)) {
                    (Some(a), Some(b)) => SelectedRange::Range(a, b),
                    _ => SelectedRange::None,
                }
            }
            Select::Range(a, b) => match (
                doc.find_text_position(a.x, a.y)
                    .filter(|(id, _)| self.roots.contains(id)),
                doc.find_text_position(b.x, b.y)
                    .filter(|(id, _)| self.roots.contains(id))
                    .or_else(|| nearest_position(doc, &self.roots, b)),
            ) {
                (Some((an, ai)), Some((bn, bi))) => {
                    SelectedRange::Range(Endpoint::new(doc, an, ai), Endpoint::new(doc, bn, bi))
                }
                _ => SelectedRange::None,
            },
        };
        self.selected()
    }
    fn ranges(&self) -> Vec<(blitz_dom::NodeId, usize, usize)> {
        match &self.selected_range {
            SelectedRange::None => Vec::new(),
            SelectedRange::All => self
                .roots
                .iter()
                .filter_map(|&id| {
                    Some((
                        id,
                        0,
                        self.document
                            .get_node(id)?
                            .element_data()?
                            .inline_layout_data
                            .as_ref()?
                            .text
                            .len(),
                    ))
                })
                .collect(),
            SelectedRange::Range(anchor, focus) => {
                let (Some((a, ai)), Some((b, bi))) = (
                    anchor.resolve(&self.document),
                    focus.resolve(&self.document),
                ) else {
                    return Vec::new();
                };
                let (Some(ap), Some(bp)) = (
                    self.roots.iter().position(|&id| id == a),
                    self.roots.iter().position(|&id| id == b),
                ) else {
                    return Vec::new();
                };
                let (lo, start, hi, end) = if (ap, ai) <= (bp, bi) {
                    (ap, ai, bp, bi)
                } else {
                    (bp, bi, ap, ai)
                };
                self.roots[lo..=hi]
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &id)| {
                        let text = &self
                            .document
                            .get_node(id)?
                            .element_data()?
                            .inline_layout_data
                            .as_ref()?
                            .text;
                        let from = if i == 0 { start } else { 0 };
                        let to = if i == hi - lo { end } else { text.len() };
                        (from < to && text.get(from..to).is_some()).then_some((id, from, to))
                    })
                    .collect()
            }
        }
    }
    fn selected(&self) -> Selected {
        let doc = &self.document;
        // The pinned engine orders an anonymous child by its DOM parent;
        // a trailing body block can therefore sort before a preceding table.
        // Use layout-root order for the range, Parley for cluster positions
        // and highlight rectangles, and preserve paragraph boundaries.
        let mut blocks = Vec::new();
        let mut rects = Vec::new();
        for (id, start, end) in self.ranges() {
            let Some(node) = doc.get_node(id) else {
                continue;
            };
            let Some(inline) = node
                .element_data()
                .and_then(|e| e.inline_layout_data.as_ref())
            else {
                continue;
            };
            let layout = &inline.layout;
            if let Some(text) = inline.text.get(start..end) {
                blocks.push(text);
            }
            let scale = layout.scale() as f64;
            let origin = node.absolute_position(0., 0.);
            let box_layout = node.final_layout();
            let ox = origin.x + box_layout.padding.left + box_layout.border.left;
            let oy = origin.y + box_layout.padding.top + box_layout.border.top;
            let selection = ParleySelection::new(
                Cursor::from_byte_index(layout, start, Affinity::Downstream),
                Cursor::from_byte_index(layout, end, Affinity::Downstream),
            );
            selection.geometry_with(layout, |r, _| {
                rects.push(Rect {
                    x: ox + (r.x0 / scale) as f32,
                    y: oy + (r.y0 / scale) as f32,
                    width: ((r.x1 - r.x0) / scale) as f32,
                    height: ((r.y1 - r.y0) / scale) as f32,
                })
            });
        }
        let convert = |id, offset| {
            let index = self.roots.iter().position(|&candidate| candidate == id)?;
            let key = self.accessible.get(index)?.key;
            let text = &doc
                .get_node(id)?
                .element_data()?
                .inline_layout_data
                .as_ref()?
                .text;
            Some(TextPosition {
                key,
                character: text.get(..offset)?.chars().count(),
            })
        };
        let accessible = match &self.selected_range {
            SelectedRange::None => None,
            SelectedRange::All => {
                let first = self.roots.first().copied();
                let last = self.roots.last().copied();
                first.zip(last).and_then(|(a, b)| {
                    Some((
                        convert(a, 0)?,
                        convert(
                            b,
                            doc.get_node(b)?
                                .element_data()?
                                .inline_layout_data
                                .as_ref()?
                                .text
                                .len(),
                        )?,
                    ))
                })
            }
            SelectedRange::Range(a, b) => a
                .resolve(doc)
                .zip(b.resolve(doc))
                .and_then(|((a, ai), (b, bi))| Some((convert(a, ai)?, convert(b, bi)?))),
        };
        Selected {
            text: blocks.join("\n"),
            rects,
            accessible,
        }
    }
}

enum SelectedRange {
    None,
    All,
    Range(Endpoint, Endpoint),
}
struct Endpoint {
    id: blitz_dom::NodeId,
    anonymous: Option<(blitz_dom::NodeId, usize)>,
    offset: usize,
    text_hash: u64,
}
fn text_hash(text: &str) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hash);
    hash.finish()
}
impl Endpoint {
    fn new(doc: &HtmlDocument, id: blitz_dom::NodeId, offset: usize) -> Self {
        let node = doc.get_node(id).unwrap();
        let anonymous = if node.is_anonymous() {
            node.parent.and_then(|parent| {
                let parent_node = doc.get_node(parent)?;
                let children = parent_node.layout_children.borrow();
                Some((
                    parent,
                    children
                        .as_ref()?
                        .iter()
                        .filter(|&&id| doc.get_node(id).is_some_and(|n| n.is_anonymous()))
                        .position(|&candidate| candidate == id)?,
                ))
            })
        } else {
            None
        };
        let text = &node
            .element_data()
            .unwrap()
            .inline_layout_data
            .as_ref()
            .unwrap()
            .text;
        Self {
            id,
            anonymous,
            offset,
            text_hash: text_hash(text),
        }
    }
    fn resolve(&self, doc: &HtmlDocument) -> Option<(blitz_dom::NodeId, usize)> {
        let id = if let Some((parent, index)) = self.anonymous {
            let node = doc.get_node(parent)?;
            let children = node.layout_children.borrow();
            *children
                .as_ref()?
                .iter()
                .filter(|&&id| doc.get_node(id).is_some_and(|n| n.is_anonymous()))
                .nth(index)?
        } else {
            self.id
        };
        let text = &doc
            .get_node(id)?
            .element_data()?
            .inline_layout_data
            .as_ref()?
            .text;
        (text_hash(text) == self.text_hash && text.is_char_boundary(self.offset))
            .then_some((id, self.offset))
    }
}
fn visible_text_roots(doc: &HtmlDocument) -> Vec<blitz_dom::NodeId> {
    let mut roots = Vec::new();
    let mut pending = vec![doc.root_element().id];
    while let Some(id) = pending.pop() {
        let Some(node) = doc.get_node(id) else {
            continue;
        };
        if node
            .primary_styles()
            .is_some_and(|s| s.get_effects().opacity == 0.)
            || doc.resolved_style_value(id, "display") == "none"
        {
            continue;
        }
        let layout = node.final_layout();
        if layout.size.height == 0.
            && matches!(
                doc.resolved_style_value(id, "overflow-y").as_str(),
                "hidden" | "clip" | "auto" | "scroll"
            )
        {
            continue;
        }
        if let Some(inline) = node
            .element_data()
            .and_then(|e| e.inline_layout_data.as_ref())
        {
            let visibility = doc.resolved_style_value(id, "visibility");
            if !inline.text.trim().is_empty()
                && inline.layout.width() > 0.
                && inline.layout.height() > 0.
                && !matches!(visibility.as_str(), "hidden" | "collapse")
            {
                roots.push(id);
            }
        }
        if let Some(children) = node.layout_children.borrow().as_ref() {
            pending.extend(children.iter().rev().copied());
        }
    }
    roots
}

// Pointer capture may finish in the margin or outside the page. Choose the
// closest laid-out text block, then let Parley resolve its cluster boundary.
fn nearest_position(
    doc: &HtmlDocument,
    roots: &[blitz_dom::NodeId],
    point: Point,
) -> Option<(blitz_dom::NodeId, usize)> {
    roots
        .iter()
        .copied()
        .filter_map(|id| {
            let node = doc.get_node(id)?;
            let inline = node.element_data()?.inline_layout_data.as_ref()?;
            if inline.text.is_empty() {
                return None;
            }
            let layout = node.final_layout();
            let origin = node.absolute_position(0., 0.);
            let x = origin.x + layout.padding.left + layout.border.left;
            let y = origin.y + layout.padding.top + layout.border.top;
            let w = inline.layout.width() / inline.layout.scale();
            let h = inline.layout.height() / inline.layout.scale();
            let cx = (point.x - x).clamp(0., w);
            let cy = (point.y - y).clamp(0., (h - 0.1).max(0.));
            let distance = (point.x - x - cx).powi(2) + (point.y - y - cy).powi(2);
            Some((distance, id, node.text_offset_at_point(cx, cy)?))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id, offset)| (id, offset))
}

#[cfg(test)]
mod tests {
    use super::{
        image_sources, render, Point, ReaderViewport, Select, Session, TextPosition, Worker,
    };
    use crate::platform::DesktopPlatform;
    use std::sync::Arc;
    use std::{
        io::{Read, Write},
        time::Duration,
    };

    fn session(html: &str, width: u32, scale: f32) -> Session {
        Session::new(
            html.into(),
            ReaderViewport {
                width,
                height: 640,
                scale,
            },
            false,
            Arc::new(DesktopPlatform::default()),
        )
    }
    fn receive<T>(rx: async_channel::Receiver<T>) -> T {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            match rx.try_recv() {
                Ok(value) => return value,
                Err(async_channel::TryRecvError::Closed) => {
                    panic!("Reader job was unexpectedly cancelled")
                }
                Err(_) => {
                    assert!(std::time::Instant::now() < deadline, "Reader job timed out");
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }
    fn pixel() -> Vec<u8> {
        let pixels = (0..64 * 96).flat_map(|_| [0u8, 128, 0, 255]).collect();
        super::encode_png(pixels, 64, 96).unwrap()
    }

    #[test]
    fn dpi_keeps_css_width_and_precise_unicode_selection() {
        let html="<style>body{margin:0}p{margin:0;padding:12px;font:20px 'Microsoft YaHei'}</style><p id='a'>Alpha 中文 Beta</p><p>第二行 👩🏽‍💻 Café</p><p style='display:none'>hidden-secret</p>";
        for scale in [1., 1.5, 2.] {
            let mut s = session(html, 360, scale);
            let rendered = s.paint().unwrap();
            assert_eq!(
                rendered.viewport.physical_size(),
                ((360. * scale) as u32, (640. * scale) as u32)
            );
            assert_eq!(s.document.viewport().logical_size().0, 360.);
            let selected = s.select(Select::All);
            assert_eq!(selected.text, "Alpha 中文 Beta\n第二行 👩🏽‍💻 Café");
            assert!(selected.rects.len() >= 2);
            let id = s.document.query_selector_all("#a").unwrap()[0];
            s.document.set_text_selection(id, 6, id, 12);
            let inline = s
                .document
                .get_node(id)
                .unwrap()
                .element_data()
                .unwrap()
                .inline_layout_data
                .as_ref()
                .unwrap();
            assert_eq!(&inline.text[6..12], "中文");
            // Obtain exactly the engine's two-glyph geometry, then hit it in
            // both drag directions using window/CSS coordinates at every DPI.
            let layout = &inline.layout;
            let sel = super::ParleySelection::new(
                super::Cursor::from_byte_index(layout, 6, super::Affinity::Downstream),
                super::Cursor::from_byte_index(layout, 12, super::Affinity::Downstream),
            );
            let mut r = None;
            sel.geometry_with(layout, |rect, _| r = Some(rect));
            let r = r.unwrap();
            let factor = layout.scale() as f64;
            let origin = s.document.get_node(id).unwrap().absolute_position(0., 0.);
            let a = Point {
                x: origin.x + 12. + (r.x0 / factor) as f32 + 0.1,
                y: origin.y + 12. + ((r.y0 + r.y1) / 2. / factor) as f32,
            };
            let b = Point {
                x: origin.x + 12. + (r.x1 / factor) as f32 - 0.1,
                y: a.y,
            };
            for (a, b) in [(a, b), (b, a)] {
                let selected = s.select(Select::Range(a, b));
                assert_eq!(selected.text, "中文", "scale {scale}");
                assert!(!selected.rects.is_empty());
            }
        }
    }
    #[test]
    fn actual_width_reflows_and_fixed_tables_keep_horizontal_content() {
        let html="<style>body{margin:0}@media(max-width:400px){#box{width:160px!important}}</style><div id='box' style='width:320px;height:80px'>Text</div>";
        let mut s = session(html, 720, 1.);
        s.paint().unwrap();
        let id = s.document.query_selector_all("#box").unwrap()[0];
        assert_eq!(
            s.document.get_node(id).unwrap().final_layout().size.width,
            320.
        );
        s.resize(ReaderViewport {
            width: 360,
            height: 640,
            scale: 2.,
        });
        s.paint().unwrap();
        assert_eq!(
            s.document.get_node(id).unwrap().final_layout().size.width,
            160.
        );
        let mut fixed=session("<body style='margin:0'><table style='width:640px'><tr><td>Tail content</td></tr></table></body>",360,1.);
        assert!(
            fixed.paint().unwrap().width >= 640.,
            "A fixed-width mail must be horizontally scrollable, not clipped"
        );
    }
    #[test]
    fn image_sources_follow_html_parsing_and_raster_mime() {
        let sources=image_sources("<IMG SRC = 'https://example.test/p?a=1&amp;b=2'><img data-src='https://example.test/tracker'><img src=https://example.test/unquoted><!-- <img src='https://example.test/comment'> -->");
        assert!(sources.contains("https://example.test/p?a=1&b=2"));
        assert!(sources.contains("https://example.test/unquoted"));
        assert_eq!(sources.len(), 2);
        assert!(super::valid_image("image/png", &pixel()));
        assert!(!super::valid_image("image/jpeg", &pixel()));
        assert!(!super::valid_image(
            "image/svg+xml",
            b"<svg><image href='http://example.test'/></svg>"
        ));
    }
    #[test]
    fn decoded_image_budget_rejects_small_compressed_images_when_full() {
        let bytes = pixel();
        let pixels = super::image_pixels("image/png", &bytes).unwrap();
        let mut state = super::NetState {
            pixels: super::MAX_SURFACE_PIXELS - pixels + 1,
            ..Default::default()
        };
        assert!(state
            .finish("too-many-pixels".into(), Some((bytes.clone(), pixels)))
            .is_empty());
        assert_eq!(state.bytes, 0);
        state.pixels = super::MAX_SURFACE_PIXELS - pixels;
        assert!(!state
            .finish("fits".into(), Some((bytes, pixels)))
            .is_empty());
        assert_eq!(state.pixels, super::MAX_SURFACE_PIXELS);
    }

    #[test]
    fn selection_orders_nested_tables_before_a_trailing_anonymous_block() {
        let html="<body style='margin:0'><div style='opacity:0'>Invisible preview</div><table><tr><td>Header 中文<table><tr><td>Nested cell</td></tr></table>End of cell</td></tr></table>Footer 中文</body>";
        let mut s = session(html, 360, 1.);
        s.paint().unwrap();
        let selected = s.select(Select::All);
        assert_eq!(
            selected.text,
            "Header 中文\nNested cell\nEnd of cell\nFooter 中文"
        );
        let first = selected.rects.first().unwrap();
        let last = selected.rects.last().unwrap();
        let a = Point {
            x: first.x + 0.1,
            y: first.y + first.height / 2.,
        };
        let b = Point {
            x: last.x + last.width - 0.1,
            y: last.y + last.height / 2.,
        };
        assert_eq!(s.select(Select::Range(a, b)).text, selected.text);
        assert_eq!(s.select(Select::Range(b, a)).text, selected.text);
        assert_eq!(
            s.select(Select::Range(b, Point { x: -20., y: 0. })).text,
            selected.text,
            "A margin drag must not snap to invisible preview text"
        );
        s.resize(ReaderViewport {
            width: 600,
            height: 640,
            scale: 2.,
        });
        assert_eq!(s.paint().unwrap().selection.unwrap().text, selected.text);
    }
    #[test]
    fn slow_images_do_not_block_switching_mail_or_leak_old_selection() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let (accepted, wait) = std::sync::mpsc::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut socket = loop {
                if let Ok((socket, _)) = listener.accept() {
                    break socket;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut request = [0; 2048];
            socket.read(&mut request).unwrap();
            accepted.send(()).unwrap();
            gate.recv_timeout(Duration::from_secs(10)).unwrap();
            let bytes = pixel();
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).unwrap();
            socket.write_all(&bytes).unwrap();
        });
        let worker = Worker::new();
        let platform = Arc::new(DesktopPlatform::default());
        let old = worker.load(
            1,
            "old".into(),
            format!("<img src='http://{address}/image.png'><p>old</p>"),
            Default::default(),
            true,
            platform.clone(),
        );
        wait.recv_timeout(Duration::from_secs(10)).unwrap();
        // Body must be available while the image response is still held.
        assert!(receive(old.clone()).unwrap().source.contains("<p>old</p>"));
        let skipped = worker.load(
            2,
            "skipped".into(),
            "<p>skipped</p>".into(),
            Default::default(),
            false,
            platform.clone(),
        );
        let latest = worker.load(
            3,
            "latest".into(),
            "<p>Latest 中文</p>".into(),
            Default::default(),
            false,
            platform,
        );
        release.send(()).unwrap();
        server.join().unwrap();
        assert_eq!(
            receive(latest).unwrap().source.as_ref(),
            "<p>Latest 中文</p>"
        );
        assert!(matches!(
            old.try_recv(),
            Err(async_channel::TryRecvError::Closed)
        ));
        drop(skipped); // A completed intermediate frame is harmless; only the current generation is consumed.
        assert_eq!(receive(worker.select(3, Select::All)).text, "Latest 中文");
        worker.clear(4);
        assert!(matches!(
            worker.select(3, Select::All).try_recv(),
            Err(async_channel::TryRecvError::Closed)
        ));
    }

    #[test]
    fn async_images_reflow_and_resize_reuses_the_downloaded_document() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let (release, gate) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            let mut socket = loop {
                if let Ok((socket, _)) = listener.accept() {
                    break socket;
                }
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(5));
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            let mut request = [0; 2048];
            socket.read(&mut request).unwrap();
            gate.recv_timeout(Duration::from_secs(10)).unwrap();
            let bytes = pixel();
            write!(socket,"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).unwrap();
            socket.write_all(&bytes).unwrap();
        });
        let worker = Worker::new();
        let platform = Arc::new(DesktopPlatform::default());
        let html=format!("<body style='margin:0'><div style='height:700px'>Header</div><img src='http://{address}/image.png'><p>Footer 中文</p></body>");
        let frames = worker.load(
            1,
            "same-mail".into(),
            html.clone(),
            Default::default(),
            true,
            platform.clone(),
        );
        let before = receive(frames.clone()).unwrap();
        let selected = receive(worker.select(1, Select::All));
        release.send(()).unwrap();
        server.join().unwrap();
        let after = receive(frames).unwrap();
        assert_eq!(after.selection.as_ref().unwrap().text, selected.text);
        assert!(
            after.selection.as_ref().unwrap().rects.last().unwrap().y
                >= selected.rects.last().unwrap().y + 80.,
            "Selection geometry did not follow image reflow"
        );
        assert!(
            after.height >= before.height + 80.,
            "Downloaded image did not reflow the mail"
        );
        // Server is closed. A second parse/download would lose the image.
        assert!(receive(worker.select(1, Select::Clear)).text.is_empty());
        let resized = receive(worker.load(
            2,
            "same-mail".into(),
            html,
            ReaderViewport {
                width: 360,
                height: 640,
                scale: 2.,
            },
            true,
            platform,
        ))
        .unwrap();
        assert!(
            (resized.height - after.height).abs() < 2.,
            "Resize failed to retain the decoded image"
        );
        assert!(
            resized.selection.as_ref().unwrap().text.is_empty(),
            "A cleared selection reappeared after reflow"
        );
        let png = image::load_from_memory(&resized.image.bytes).unwrap();
        assert_eq!(png.width(), 720);
    }

    #[test]
    fn oversized_surface_is_rejected_before_allocation_and_worker_recovers() {
        let worker = Worker::new();
        let platform = Arc::new(DesktopPlatform::default());
        assert!(receive(worker.load(
            1,
            "large".into(),
            "<div style='height:3000000px'>Complete plain alternative</div>".into(),
            Default::default(),
            false,
            platform.clone()
        ))
        .is_err());
        assert!(receive(worker.load(
            2,
            "next".into(),
            "<p>Still readable</p>".into(),
            Default::default(),
            false,
            platform
        ))
        .is_ok());
        assert_eq!(
            receive(worker.select(2, Select::All)).text,
            "Still readable"
        );
    }
    #[test]
    fn long_mail_paints_bounded_regions_and_reaches_the_tail_at_every_dpi() {
        let html="<body style='margin:0'><div style='height:20000px;background:#ff0000'>Header</div><div style='height:640px;background:#00ff00'>Tail 中文</div></body>";
        for scale in [1., 1.5, 2.] {
            let mut s = session(html, 720, scale);
            let first = s.paint().unwrap();
            assert!(first.height >= 20640.);
            assert!(first.area.height <= 1152.);
            let png = image::load_from_memory(&first.image.bytes)
                .unwrap()
                .into_rgba8();
            assert_eq!(png.get_pixel(300, 200).0, [255, 0, 0, 255]);
            let revision = first.layout_revision;
            s.scroll = Point { x: 0., y: 20000. };
            let tail = s.paint_region(false).unwrap();
            assert_eq!(tail.layout_revision, revision);
            assert!(
                Arc::ptr_eq(&first.accessible, &tail.accessible),
                "Scrolling rebuilt the accessible document"
            );
            assert!(tail.selection.is_none());
            assert!(tail.area.y >= 19900.);
            assert!(tail.area.height <= 1152.);
            let png = image::load_from_memory(&tail.image.bytes)
                .unwrap()
                .into_rgba8();
            assert_eq!(
                png.get_pixel((300. * scale) as u32, (100. * scale) as u32)
                    .0,
                [0, 255, 0, 255]
            );
            let selected = s.select(Select::All);
            assert_eq!(selected.text, "Header\nTail 中文");
            assert!(selected.rects.last().unwrap().y >= 20000.);
        }
    }
    #[test]
    fn accessible_selection_maps_unicode_positions_and_rejects_stale_keys() {
        let mut s = session("<p>Start 中文 👩🏽‍💻</p><p>End Café</p>", 360, 1.);
        s.paint().unwrap();
        let a = TextPosition {
            key: s.accessible[0].key,
            character: 6,
        };
        let b = TextPosition {
            key: s.accessible[1].key,
            character: 3,
        };
        let selected = s.select(Select::Accessible(a, b));
        assert_eq!(selected.text, "中文 👩🏽‍💻\nEnd");
        assert_eq!(selected.accessible, Some((a, b)));
        assert_eq!(s.select(Select::Accessible(b, a)).text, selected.text);
        assert!(s
            .select(Select::Accessible(
                TextPosition {
                    key: 0,
                    character: 0
                },
                b
            ))
            .text
            .is_empty());
        assert!(s
            .select(Select::Accessible(
                TextPosition {
                    key: a.key,
                    character: usize::MAX
                },
                b
            ))
            .text
            .is_empty());
        assert!(s.select(Select::Clear).accessible.is_none());
    }

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
