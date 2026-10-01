//! Native mail reader. Blitz renders original HTML/CSS; Kit's TextView hosts
//! plain and translated content. Selection, scrolling and clipboard belong to
//! GPUI; document layout and Unicode cluster hit testing belong to Blitz.
use gpui_kit::{base::TextView, prelude::FluentBuilder as _, *};
use lightmail_core::ReaderContent;
use std::sync::Arc;

const INK: u32 = 0x202724;
const MUTED: u32 = 0x69736e;
const ACCENT: u32 = 0x226451;
const LINE: u32 = 0xe7ebe8;
pub const PLACEHOLDER: &str = "icons/image-off.svg";
// Short bodies lay out in full inside the pane's scroll area. Long ones use
// TextView's virtualized viewport, so layout cost stays bounded.
const VIRTUAL_BYTES: usize = 8 * 1024;

/// Reader content with shared strings, so rendering never copies a body.
pub enum Document {
    Markdown(SharedString),
    Html(SharedString),
    Blitz(Arc<crate::blitz_reader::Rendered>),
    Bilingual(Vec<(SharedString, SharedString)>),
}

impl From<ReaderContent> for Document {
    fn from(content: ReaderContent) -> Self {
        match content {
            ReaderContent::Markdown(text) => Self::Markdown(text.into()),
            ReaderContent::Html(text) => Self::Html(text.into()),
            ReaderContent::Bilingual(pairs) => Self::Bilingual(
                pairs
                    .into_iter()
                    .map(|(source, target)| (source.into(), target.into()))
                    .collect(),
            ),
        }
    }
}

/// Resolves every document image, for painting and for size measurement.
/// A message URI is never handed to GPUI's default loader.
pub type Images = Arc<dyn Fn(&SharedUri) -> ImageSource + Send + Sync>;

pub fn blocked_images() -> Images {
    Arc::new(|_| ImageSource::from(PLACEHOLDER))
}

pub fn allowed_link(value: &str) -> bool {
    !value.chars().any(char::is_control)
        && url::Url::parse(value)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "mailto"))
}

fn color(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn body(
    id: SharedString,
    source: SharedString,
    html: bool,
    images: Images,
    fill: bool,
) -> TextView {
    let view = if html {
        TextView::html(id, source)
    } else {
        TextView::markdown(id, source)
    };
    view.selectable(true)
        .scrollable(fill)
        .w_full()
        .when(fill, |view| view.h_full())
        .text_size(px(15.))
        .line_height(relative(1.65))
        .style(
            gpui_kit::base::text::TextViewStyle::default()
                .with_foreground(color(INK))
                .with_link(color(ACCENT))
                .with_muted_foreground(color(MUTED))
                .with_border(color(LINE))
                .with_paragraph_gap(rems(1.)),
        )
        .image_source(move |uri| images(uri))
        .on_link_click(|link, _, _, cx| {
            if allowed_link(link) {
                // Parsing validates only; signed URLs are opened exactly as written.
                cx.open_url(link.as_str());
            }
        })
}

/// `key` names the message and reading mode, so no text state leaks between them.
pub fn view(
    key: &str,
    document: &Document,
    images: Images,
    state: &crate::app::MailDesktop,
    cx: &Context<crate::app::MailDesktop>,
) -> AnyElement {
    let scroll = || div().id("reader-scroll").size_full().overflow_y_scroll();
    match document {
        Document::Markdown(text) | Document::Html(text) => {
            let html = matches!(document, Document::Html(_));
            let fill = text.len() > VIRTUAL_BYTES;
            let view = body(
                format!("reader:{key}").into(),
                text.clone(),
                html,
                images,
                fill,
            );
            if fill {
                div().size_full().child(view).into_any_element()
            } else {
                scroll().child(view).into_any_element()
            }
        }
        Document::Blitz(rendered) => {
            let mut surface = div()
                .id(format!("reader-blitz:{key}"))
                .relative()
                .w(px(rendered.width))
                .h(px(rendered.height))
                .track_focus(&state.reader_focus)
                .key_context("BlitzReader")
                .on_action(
                    cx.listener(|s, _: &crate::shortcuts::CopyReaderSelection, _, cx| {
                        s.copy_reader_selection(cx)
                    }),
                )
                .on_action(
                    cx.listener(|s, _: &crate::shortcuts::SelectReaderAll, _, cx| {
                        s.select_reader(crate::blitz_reader::Select::All, cx)
                    }),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|s, e: &MouseDownEvent, w, cx| {
                        s.begin_reader_selection(e.position, w, cx)
                    }),
                )
                .on_mouse_move(cx.listener(|s, e: &MouseMoveEvent, _, cx| {
                    if e.pressed_button == Some(MouseButton::Left) {
                        s.move_reader_selection(e.position, cx);
                    }
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|s, e: &MouseUpEvent, _, cx| {
                        s.end_reader_selection(e.position, true, cx)
                    }),
                )
                .on_mouse_up_out(
                    MouseButton::Left,
                    cx.listener(|s, e: &MouseUpEvent, _, cx| {
                        s.end_reader_selection(e.position, false, cx)
                    }),
                )
                .cursor(CursorStyle::IBeam)
                .child(
                    img(ImageSource::Image(rendered.image.clone()))
                        .id(format!("reader-blitz-image:{key}"))
                        .w(px(rendered.width))
                        .h(px(rendered.height)),
                );
            for (index, link) in rendered.links.iter().enumerate() {
                surface = surface.child(
                    div()
                        .id(format!("reader-blitz-link:{key}:{index}"))
                        .absolute()
                        .left(px(link.x))
                        .top(px(link.y))
                        .w(px(link.width))
                        .h(px(link.height))
                        .bg(rgba(0x00000000))
                        .cursor_pointer(),
                );
            }
            for (index, rect) in state.reader_selection.rects.iter().enumerate() {
                surface = surface.child(
                    div()
                        .id(format!("reader-selection:{index}"))
                        .absolute()
                        .left(px(rect.x))
                        .top(px(rect.y))
                        .w(px(rect.width))
                        .h(px(rect.height))
                        .bg(rgba(0x4285f44d)),
                );
            }
            let weak = cx.entity().downgrade();
            surface = surface.child(
                canvas(
                    move |bounds, _, cx| {
                        let _ = weak.update(cx, |s, _| s.reader_surface = bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );
            div()
                .id("reader-scroll")
                .size_full()
                .overflow_scroll()
                .track_scroll(&state.reader_scroll)
                .child(surface)
                .into_any_element()
        }
        Document::Bilingual(pairs) => {
            let mut rows = div().flex().flex_col().gap_6().w_full().pb_6();
            for (index, (source, target)) in pairs.iter().enumerate() {
                rows = rows.child(
                    div()
                        .flex()
                        .gap_6()
                        .w_full()
                        .child(div().flex_1().min_w_0().child(body(
                            format!("reader:{key}:{index}:source").into(),
                            source.clone(),
                            false,
                            images.clone(),
                            false,
                        )))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .pl_6()
                                .border_l_1()
                                .border_color(rgb(LINE))
                                .child(body(
                                    format!("reader:{key}:{index}:target").into(),
                                    target.clone(),
                                    false,
                                    images.clone(),
                                    false,
                                )),
                        ),
                );
            }
            scroll().child(rows).into_any_element()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::allowed_link;

    #[test]
    fn links_leave_the_app_only_for_web_and_mail() {
        for link in [
            "https://example.test/action?t=a%2Bb%3D",
            "http://example.test/",
            "mailto:reader@example.test",
        ] {
            assert!(allowed_link(link), "{link}");
        }
        for link in [
            "file:///C:/private.txt",
            "javascript:alert(1)",
            "data:text/html,hello",
            "ms-settings:privacy",
            "https://example.test/\r\n",
            "relative/path",
        ] {
            assert!(!allowed_link(link), "{link}");
        }
    }
}
