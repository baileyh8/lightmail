//! Portable provider defaults and safe reader documents used by both native shells.
use crate::{composition::ExportMode, models::*, translation::*};

#[derive(Clone, Debug, uniffi::Record)]
pub struct ProviderPreset {
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub auth_kind: String,
    pub sent_mode: String,
}
#[uniffi::export]
pub fn provider_preset(provider: String) -> ProviderPreset {
    let (imap, smtp, auth, sent) = match provider.as_str() {
        "gmail" => ("imap.gmail.com", "smtp.gmail.com", "oauth", "server"),
        "qq" => ("imap.qq.com", "smtp.qq.com", "password", "server"),
        "163" => ("imap.163.com", "smtp.163.com", "password", "server"),
        _ => ("", "", "password", "server"),
    };
    ProviderPreset {
        imap_host: imap.into(),
        imap_port: 993,
        smtp_host: smtp.into(),
        smtp_port: 465,
        auth_kind: auth.into(),
        sent_mode: sent.into(),
    }
}

/// Default account colors, matching the existing macOS settings screen.
pub fn provider_color(provider: &str) -> &'static str {
    match provider {
        "gmail" => "#226451",
        "163" => "#BC795F",
        "qq" => "#C19944",
        _ => "#6687B7",
    }
}

/// Input for native readers that render Markdown or restricted HTML without a
/// browser document. Translation validation and block pairing stay in the core.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReaderContent {
    Markdown(String),
    /// Already sanitized and size-bounded when the body was converted.
    Html(String),
    /// Source and translated Markdown for each translation block.
    Bilingual(Vec<(String, String)>),
}

pub fn reader_content(
    body: &MailBody,
    translation: Option<&TranslationResult>,
    mode: ExportMode,
    rich: bool,
) -> Result<ReaderContent> {
    match mode {
        ExportMode::Original => {
            if rich && !body.html.is_empty() {
                return Ok(ReaderContent::Html(body.html.clone()));
            }
            let markdown = if body.markdown.trim().is_empty() {
                &body.text
            } else {
                &body.markdown
            };
            markdown_budget(markdown)?;
            Ok(ReaderContent::Markdown(markdown.clone()))
        }
        ExportMode::Translated => {
            let t = translation.ok_or_else(|| fail("译文尚未完成"))?;
            crate::composition::validate_translation(body, t)?;
            let markdown = translation_markdown(t.clone());
            markdown_budget(&markdown)?;
            Ok(ReaderContent::Markdown(markdown))
        }
        ExportMode::Bilingual => {
            let t = translation.ok_or_else(|| fail("译文尚未完成"))?;
            crate::composition::validate_translation(body, t)?;
            let mut size = 0usize;
            let mut pairs = Vec::new();
            for block in translation_blocks(body.markdown.clone()) {
                let target = t
                    .blocks
                    .iter()
                    .find(|b| b.id == block.id)
                    .map(|b| b.text.clone())
                    .unwrap_or_default();
                markdown_budget(&block.text)?;
                markdown_budget(&target)?;
                size = size.saturating_add(block.text.len() + target.len());
                if size > 8 * 1024 * 1024 {
                    return Err(fail("双语正文超过显示上限"));
                }
                pairs.push((block.text, target));
            }
            Ok(ReaderContent::Bilingual(pairs))
        }
    }
}

#[uniffi::export]
pub fn reader_document(html: String, load_images: bool) -> String {
    let sources = if load_images {
        "https: http:"
    } else {
        "'none'"
    };
    let image_style = if load_images {
        ".lightmail-image-link-label{display:none!important}"
    } else {
        "img{display:none!important}.lightmail-image-link-label{display:inline-block!important;padding:10px 16px!important;border:1px solid currentColor!important;border-radius:5px!important;font:14px sans-serif!important;color:#226451!important;background:#f2f7f5!important}"
    };
    format!("<!doctype html><html><head><meta charset='utf-8'><meta http-equiv='Content-Security-Policy' content=\"default-src 'none'; style-src 'unsafe-inline'; img-src {sources}; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'\"><style>:root{{color-scheme:light}}body{{font:15px -apple-system,BlinkMacSystemFont,'Segoe UI','Microsoft YaHei UI',sans-serif;line-height:1.65;color:#202724;margin:0;overflow-wrap:anywhere}}table{{max-width:100%}}pre{{white-space:pre-wrap}}a{{color:#226451}}blockquote{{border-left:2px solid #e7ebe8;margin-left:0;padding-left:16px}}.lightmail-empty-link-label{{display:inline-block!important;padding:10px 16px!important;border:1px solid currentColor!important;border-radius:5px!important;color:#226451!important;background:#f2f7f5!important}}{image_style}</style></head><body>{html}</body></html>")
}
fn markdown_options() -> pulldown_cmark::Options {
    pulldown_cmark::Options::ENABLE_TABLES | pulldown_cmark::Options::ENABLE_STRIKETHROUGH
}
// Enforce the same finite work budget as MIME conversion before any reader,
// browser or native, allocates a document for this Markdown.
fn markdown_budget(markdown: &str) -> Result<()> {
    use pulldown_cmark::{Event, Parser};
    if markdown.len() > 4 * 1024 * 1024 {
        return Err(fail("正文超过显示上限"));
    }
    let mut depth = 0usize;
    for (count, event) in Parser::new_ext(markdown, markdown_options()).enumerate() {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        if count >= 50_000 || depth > 64 {
            return Err(fail("正文结构过于复杂"));
        }
    }
    Ok(())
}
fn markdown_html(markdown: &str) -> Result<String> {
    use pulldown_cmark::{Event, Parser};
    use std::io::Write;
    markdown_budget(markdown)?;
    let options = markdown_options();
    struct Bounded(Vec<u8>);
    impl Write for Bounded {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.0.len().saturating_add(bytes.len()) > 8 * 1024 * 1024 {
                return Err(std::io::Error::other("reader output limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Bounded(Vec::new());
    let events = Parser::new_ext(markdown, options).map(|event| match event {
        Event::Html(text) | Event::InlineHtml(text) => Event::Text(text),
        other => other,
    });
    pulldown_cmark::html::write_html_io(&mut output, events)
        .map_err(|_| fail("正文超过显示上限"))?;
    Ok(crate::mime::clean_html(
        &String::from_utf8(output.0).map_err(fail)?,
    ))
}
#[uniffi::export]
pub fn render_body_document(
    body: MailBody,
    translation: Option<TranslationResult>,
    mode: ExportMode,
    load_images: bool,
) -> Result<String> {
    let html = match mode {
        ExportMode::Original => {
            if body.html.is_empty() {
                markdown_html(&body.markdown)?
            } else {
                body.html
            }
        }
        ExportMode::Translated => {
            let t = translation.ok_or_else(|| fail("译文尚未完成"))?;
            crate::composition::validate_translation(&body, &t)?;
            markdown_html(&translation_markdown(t))?
        }
        ExportMode::Bilingual => {
            let t = translation.ok_or_else(|| fail("译文尚未完成"))?;
            crate::composition::validate_translation(&body, &t)?;
            let mut html=String::from("<style>.bilingual{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:24px;margin:0 0 24px}.bilingual>div:nth-child(2){border-left:1px solid #e7ebe8;padding-left:24px}@media(max-width:640px){.bilingual{grid-template-columns:1fr}}</style>");
            for block in translation_blocks(body.markdown) {
                let target = &t.blocks.iter().find(|b| b.id == block.id).unwrap().text;
                html.push_str(&format!(
                    "<section class='bilingual'><div>{}</div><div>{}</div></section>",
                    markdown_html(&block.text)?,
                    markdown_html(target)?
                ));
                if html.len() > 8 * 1024 * 1024 {
                    return Err(fail("双语正文超过显示上限"));
                }
            }
            html
        }
    };
    Ok(reader_document(html, load_images))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn markdown_budget_and_raw_html() {
        assert!(markdown_html(&"x".repeat(4 * 1024 * 1024 + 1)).is_err());
        assert!(markdown_html(&format!("{} x", "> ".repeat(100))).is_err());
        let html =
            markdown_html("<script>alert(1)</script>\n\n**Safe** [link](https://example.com)")
                .unwrap();
        assert!(!html.contains("<script>"));
        assert!(html.contains("<strong>Safe</strong>"));
    }
    #[test]
    fn native_reader_content_follows_mode_and_validation() {
        let body = MailBody {
            message_id: "m".into(),
            text: "plain".into(),
            markdown: "# Title\n\nFirst\n\nSecond".into(),
            html: "<p>First</p>".into(),
            attachments: vec![],
            content_hash: "hash".into(),
        };
        assert_eq!(
            reader_content(&body, None, ExportMode::Original, true).unwrap(),
            ReaderContent::Html("<p>First</p>".into())
        );
        assert_eq!(
            reader_content(&body, None, ExportMode::Original, false).unwrap(),
            ReaderContent::Markdown(body.markdown.clone())
        );
        let mut plain = body.clone();
        plain.markdown.clear();
        plain.html.clear();
        assert_eq!(
            reader_content(&plain, None, ExportMode::Original, true).unwrap(),
            ReaderContent::Markdown("plain".into())
        );
        assert!(reader_content(&body, None, ExportMode::Translated, false).is_err());
        let blocks = translation_blocks(body.markdown.clone());
        let translation = TranslationResult {
            subject: "标题".into(),
            blocks: blocks
                .iter()
                .map(|b| TranslationBlock {
                    id: b.id,
                    text: format!("译 {}", b.text),
                })
                .collect(),
            source_hash: "hash".into(),
            model: "fixture".into(),
            input_tokens: None,
            output_tokens: None,
        };
        assert_eq!(
            reader_content(&body, Some(&translation), ExportMode::Translated, false).unwrap(),
            ReaderContent::Markdown(translation_markdown(translation.clone()))
        );
        match reader_content(&body, Some(&translation), ExportMode::Bilingual, false).unwrap() {
            ReaderContent::Bilingual(pairs) => {
                assert_eq!(pairs.len(), blocks.len());
                assert_eq!(pairs[1], ("First".to_string(), "译 First".to_string()));
            }
            other => panic!("unexpected reader content {other:?}"),
        }
        let mut stale = translation;
        stale.source_hash = "old".into();
        assert!(reader_content(&body, Some(&stale), ExportMode::Bilingual, false).is_err());
        let mut huge = body;
        huge.markdown = "x".repeat(4 * 1024 * 1024 + 1);
        huge.html.clear();
        assert!(reader_content(&huge, None, ExportMode::Original, true).is_err());
        assert_eq!(provider_color("qq"), "#C19944");
        assert_eq!(provider_color("custom"), "#6687B7");
    }
    #[test]
    fn document_policy_blocks_execution_and_keeps_layout() {
        let doc = reader_document("<table><tr><td>Hello</td></tr></table>".into(), false);
        assert!(doc.contains("img-src 'none'"));
        assert!(doc.contains("default-src 'none'"));
        assert!(doc.contains("<table>"));
        let doc = reader_document(String::new(), true);
        assert!(doc.contains("img-src https: http:"));
        assert!(doc.contains("frame-src 'none'"));
        assert!(
            !markdown_html("<script>alert(1)</script> [open](javascript:evil)")
                .unwrap()
                .contains("href=\"javascript:")
        );
    }
}
