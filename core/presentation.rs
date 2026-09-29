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
// Enforce the same finite work budget as MIME conversion before allocating HTML.
fn markdown_html(markdown: &str) -> Result<String> {
    use pulldown_cmark::{Event, Options, Parser};
    use std::io::Write;
    if markdown.len() > 4 * 1024 * 1024 {
        return Err(fail("正文超过显示上限"));
    }
    let options = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    let mut depth = 0usize;
    for (count, event) in Parser::new_ext(markdown, options).enumerate() {
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        if count >= 50_000 || depth > 64 {
            return Err(fail("正文结构过于复杂"));
        }
    }
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
