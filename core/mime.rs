use crate::{models::*, store::MailEngine};
use base64::Engine;
use mail_parser::{MessageParser, MimeHeaders};
use rusqlite::params;
use sha2::{Digest, Sha256};
pub(crate) const INLINE_IMAGE_BYTES: usize = 2 * 1024 * 1024;

pub(crate) fn inline_image_uri(mime: &str, bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > INLINE_IMAGE_BYTES {
        return None;
    }
    let mime = mime.to_ascii_lowercase();
    let valid = match mime.as_str() {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xff, 0xd8, 0xff]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        "image/bmp" => bytes.starts_with(b"BM"),
        _ => false,
    };
    valid.then(|| {
        format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        )
    })
}
fn safe_image_source(source: &str) -> bool {
    if crate::html::cid_id(source).is_some() {
        return source.len() <= 1024;
    }
    if let Some(value) = source.strip_prefix("data:") {
        let Some((mime, encoded)) = value.split_once(";base64,") else {
            return false;
        };
        if encoded.len() > INLINE_IMAGE_BYTES / 3 * 4 + 4 {
            return false;
        }
        return base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .ok()
            .is_some_and(|bytes| inline_image_uri(mime, &bytes).is_some());
    }
    url::Url::parse(source).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

pub fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

#[cfg(test)]
mod inline_tests {
    use super::*;
    const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==";
    #[test]
    fn related_mime_resolves_cid_and_keeps_only_raster_data_sources() {
        let raw = format!("MIME-Version: 1.0\r\nContent-Type: multipart/related; boundary=fixture\r\n\r\n--fixture\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>Body</p><img src='cid:logo%40fixture' alt='Logo'><img src='data:image/png;base64,{PIXEL}'><img src='data:image/svg+xml;base64,PHN2Zz48L3N2Zz4='><a href='data:text/html,bad'>bad</a>\r\n--fixture\r\nContent-Type: image/png\r\nContent-ID: <logo@fixture>\r\nContent-Transfer-Encoding: base64\r\n\r\n{PIXEL}\r\n--fixture--\r\n");
        let body = parse_body(raw.as_bytes(), "cid-fixture").unwrap();
        assert!(!body.html.contains("cid:"));
        assert_eq!(body.html.matches("data:image/png;base64,").count(), 2);
        assert!(!body.html.contains("data:image/svg"));
        assert!(!body.html.contains("href=\"data:"));
        assert!(body.markdown.contains("Body"));
        assert!(inline_image_uri("image/png", &vec![0; INLINE_IMAGE_BYTES + 1]).is_none());
        assert!(safe_image_source("data:text/html;base64,PGgxPmJhZDwvaDE+") == false);
        assert!(
            !clean_html("<img src='file:///private.png'><a href='cid:logo'>x</a>")
                .contains("src=\"file:")
        );
    }
}
pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
pub fn clean_html(html: &str) -> String {
    let prepared = crate::html::restore_link_labels(html);
    let clean = ammonia::Builder::default()
        .rm_clean_content_tags(&["style"])
        .add_clean_content_tags(&["title"])
        .add_tags(&["style", "font", "center"])
        .add_tag_attributes("img", &["src", "alt", "width", "height", "title"])
        .add_tag_attributes(
            "table",
            &[
                "width",
                "height",
                "align",
                "bgcolor",
                "border",
                "cellpadding",
                "cellspacing",
            ],
        )
        .add_tag_attributes("td", &["width", "height", "align", "valign", "bgcolor"])
        .add_tag_attributes("th", &["width", "height", "align", "valign", "bgcolor"])
        .add_tag_attributes("font", &["face", "color", "size"])
        .rm_tags(&[
            "svg", "video", "audio", "iframe", "form", "input", "button", "link", "meta", "object",
            "embed",
        ])
        .rm_generic_attributes(&["src", "srcset", "background"])
        .add_generic_attributes(&["style", "role", "class", "id"])
        .filter_style_properties(
            [
                "color",
                "background-color",
                "background",
                "font-family",
                "font-size",
                "font-weight",
                "font-style",
                "line-height",
                "letter-spacing",
                "text-align",
                "text-decoration",
                "vertical-align",
                "width",
                "max-width",
                "min-width",
                "height",
                "padding",
                "padding-top",
                "padding-bottom",
                "padding-left",
                "padding-right",
                "margin",
                "margin-top",
                "margin-bottom",
                "margin-left",
                "margin-right",
                "border",
                "border-top",
                "border-bottom",
                "border-left",
                "border-right",
                "border-color",
                "border-width",
                "border-style",
                "border-radius",
                "border-collapse",
                "border-spacing",
                "display",
                "white-space",
                "word-break",
                "overflow-wrap",
            ]
            .into_iter()
            .collect(),
        )
        .url_schemes(
            ["https", "http", "mailto", "cid", "data"]
                .into_iter()
                .collect(),
        )
        .attribute_filter(|tag, attribute, value| {
            if tag == "img" && attribute == "src" {
                return safe_image_source(value).then(|| std::borrow::Cow::Borrowed(value));
            }
            if attribute == "href"
                && !url::Url::parse(value)
                    .is_ok_and(|url| matches!(url.scheme(), "http" | "https" | "mailto"))
            {
                return None;
            }
            Some(std::borrow::Cow::Borrowed(value))
        })
        .clean(&prepared)
        .to_string();
    format!("{}{clean}", crate::html::RENDER_MARKER)
}
pub fn summary(
    raw: &[u8],
    account: &Account,
    folder: &Folder,
    uid: u32,
    validity: u32,
) -> MessageSummary {
    let mail = MessageParser::default().parse(raw);
    let from = mail.as_ref().and_then(|m| m.from()).and_then(|a| a.first());
    let addr = |a: Option<&mail_parser::Address<'_>>| {
        a.map(|a| {
            a.iter()
                .filter_map(|v| v.address.as_deref())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
    };
    let id = format!("{}:{}:{}:{}", account.id, folder.path, validity, uid);
    MessageSummary {
        id: id.clone(),
        account_id: account.id.clone(),
        folder_id: folder.id.clone(),
        uid,
        uid_validity: validity,
        canonical_id: id,
        thread_id: String::new(),
        message_id: mail
            .as_ref()
            .and_then(|m| m.message_id())
            .unwrap_or("")
            .into(),
        subject: mail
            .as_ref()
            .and_then(|m| m.subject())
            .filter(|s| !s.is_empty())
            .unwrap_or("（无主题）")
            .into(),
        from_name: from.and_then(|v| v.name.as_deref()).unwrap_or("").into(),
        from_address: from.and_then(|v| v.address.as_deref()).unwrap_or("").into(),
        reply_to_address: addr(mail.as_ref().and_then(|m| m.reply_to())),
        to_addresses: addr(mail.as_ref().and_then(|m| m.to())),
        cc_addresses: addr(mail.as_ref().and_then(|m| m.cc())),
        timestamp: mail
            .as_ref()
            .and_then(|m| m.date())
            .map(|d| d.to_timestamp())
            .unwrap_or_else(now),
        snippet: String::new(),
        unread: true,
        starred: false,
        size: raw.len() as u64,
        has_attachments: false,
    }
}
pub fn parse_body(raw: &[u8], id: &str) -> Result<MailBody> {
    let mail = MessageParser::default()
        .parse(raw)
        .ok_or_else(|| fail("无法解析邮件正文"))?;
    let mut inline = std::collections::HashMap::new();
    let mut inline_bytes = 0;
    for part in &mail.parts {
        if inline.len() >= 64 {
            break;
        }
        let Some(cid) = part.content_id() else {
            continue;
        };
        let Some(kind) = part.content_type() else {
            continue;
        };
        let bytes = part.contents();
        if inline_bytes + bytes.len() > INLINE_IMAGE_BYTES {
            continue;
        }
        let mime = format!("{}/{}", kind.ctype(), kind.subtype().unwrap_or(""));
        if let Some(uri) = inline_image_uri(&mime, bytes) {
            inline_bytes += bytes.len();
            inline
                .entry(cid.trim_matches(['<', '>']).to_owned())
                .or_insert(uri);
        }
    }
    let mut html = Vec::new();
    let mut markdown = Vec::new();
    // The parser's HTML-body sequence also includes plain text sections in multipart/mixed.
    for p in mail.html_bodies() {
        match &p.body {
            mail_parser::PartType::Html(value) => {
                let safe = clean_html(&crate::html::embed_inline_images(value, &inline));
                markdown.push(crate::markdown::from_html(&safe)?);
                html.push(safe);
            }
            mail_parser::PartType::Text(value) => {
                markdown.push(value.to_string());
                html.push(format!("<pre>{}</pre>", escape_html(value)));
            }
            _ => {}
        }
    }
    let has_html = mail
        .html_bodies()
        .any(|p| matches!(p.body, mail_parser::PartType::Html(_)));
    let safe = if has_html {
        html.join("\n")
    } else {
        String::new()
    };
    let markdown = markdown.join("\n\n");
    if markdown.len() > 8 * 1024 * 1024 || safe.len() > 8 * 1024 * 1024 {
        return Err(fail("转换后的正文超过 8 MiB，已停止加载以保护内存"));
    }
    let text = plain_text(&safe, &markdown);
    let attachments = mail
        .attachments()
        .enumerate()
        .map(|(i, p)| AttachmentInfo {
            part_id: format!("local:{}", i),
            filename: p.attachment_name().unwrap_or("附件").into(),
            mime_type: p
                .content_type()
                .map(|t| format!("{}/{}", t.ctype(), t.subtype().unwrap_or("octet-stream")))
                .unwrap_or_else(|| "application/octet-stream".into()),
            size: p.contents().len() as u64,
        })
        .collect();
    let content_hash = hash(&markdown);
    Ok(MailBody {
        message_id: id.into(),
        text,
        markdown,
        html: safe,
        attachments,
        content_hash,
    })
}
pub fn plain_text(html: &str, fallback: &str) -> String {
    if html.is_empty() {
        fallback.into()
    } else {
        mail_parser::decoders::html::html_to_text(html)
    }
}
pub fn preview(text: &str) -> String {
    let value: String = text
        .split_whitespace()
        .flat_map(|word| word.chars().chain(std::iter::once(' ')))
        .take(140)
        .collect();
    if value.trim().is_empty() {
        "（正文为空）".into()
    } else {
        value.trim().into()
    }
}
pub fn decode_folder(input: &str) -> String {
    let mut out = String::new();
    let mut iter = input.chars();
    while let Some(c) = iter.next() {
        if c != '&' {
            out.push(c);
            continue;
        }
        let seq: String = iter.by_ref().take_while(|c| *c != '-').collect();
        if seq.is_empty() {
            out.push('&');
            continue;
        }
        match base64::engine::general_purpose::STANDARD_NO_PAD.decode(seq.replace(',', "/")) {
            Ok(bytes) => {
                let units: Vec<u16> = bytes
                    .chunks_exact(2)
                    .map(|c| u16::from_be_bytes([c[0], c[1]]))
                    .collect();
                out.push_str(&String::from_utf16_lossy(&units));
            }
            Err(_) => {
                out.push('&');
                out.push_str(&seq);
                out.push('-');
            }
        }
    }
    out
}
pub fn seed_demo(engine: &MailEngine) -> Result<()> {
    if !engine.accounts()?.is_empty() {
        return Ok(());
    }
    let samples = [
        ("demo-work", "Gmail 工作", "#226451"),
        ("demo-personal", "Gmail 个人", "#6687B7"),
        ("demo-163", "163 邮箱", "#BC795F"),
        ("demo-qq", "QQ 邮箱", "#C19944"),
    ];
    for (id, name, color) in samples {
        let a = Account {
            id: id.into(),
            name: name.into(),
            address: format!("{}@example.com", id),
            provider: "demo".into(),
            imap_host: "demo.invalid".into(),
            imap_port: 993,
            smtp_host: "demo.invalid".into(),
            smtp_port: 465,
            auth_kind: "demo".into(),
            color: color.into(),
            enabled: false,
            sent_mode: "server".into(),
        };
        engine.save_account(a)?;
        for (path, title, role) in [
            ("INBOX", "收件箱", "inbox"),
            ("Sent", "已发送", "sent"),
            ("Projects", "项目协作", "custom"),
        ] {
            let f = Folder {
                id: folder_id(id, path),
                account_id: id.into(),
                path: path.into(),
                name: title.into(),
                role: role.into(),
                unread_count: 0,
                total_count: 0,
            };
            engine
                .connection()?
                .execute(
                    "INSERT INTO folders(id,account_id,path,role,data) VALUES(?1,?2,?3,?4,?5)",
                    params![
                        f.id,
                        id,
                        path,
                        role,
                        serde_json::to_string(&f).map_err(fail)?
                    ],
                )
                .map_err(fail)?;
        }
    }
    let intro="Hi Bailey,\n\nThe updated launch plan is ready for a final review.\n\nWe have simplified the onboarding flow and brought all account settings into one place. The latest draft is attached below.\n\nBefore Friday, could you take a look at:\n\n- The first-time setup experience\n- The wording in the account switcher\n- The final launch checklist\n\nA few focused comments would be perfect. We can finalize the details together tomorrow.\n\nThanks,\nEmma";
    let messages=[("demo-work","Emma Chen","emma@example.com","Q4 launch · final review",intro),
    ("demo-personal","Notion","news@example.com","Your weekly workspace digest","A few things you may have missed.\n\nYour team's workspace is ready for the week ahead.\n\nThis is a sample email for previewing Lightmail."),
    ("demo-163","林悦","lin@example.com","九月项目资料已更新","Bailey，你好：\n\n更新后的项目资料已经整理完毕，请抽空查看。关于下周的安排，我们可以在周四一起确认。\n\n谢谢！\n林悦"),
    ("demo-work","Figma","design@example.com","New comments on your design","Emma left a comment on Review:\n\nThe account switcher feels much clearer now. Could we make the empty state a little more welcoming?"),
    ("demo-qq","陈舟","chen@example.com","周五见面时间确认","下午三点可以，届时见。\n\n如果时间有变化，提前告诉我即可。"),
    ("demo-work","Linear","updates@example.com","Your team’s weekly update","The latest from your team\n\n- Account settings are ready for review.\n- The onboarding flow has been simplified.\n- Next up: polish the reading experience."),
    ("demo-personal","Maya Park","maya@example.com","A few notes from our conversation","Hi Bailey,\n\nThank you for the thoughtful feedback. We hope to share the revised draft next week, subject to the team's final review.\n\nBest,\nMaya")];
    for (i, (aid, name, address, subject, body)) in messages.iter().enumerate() {
        let a = engine.account(aid)?;
        let f = engine
            .folders(aid.to_string())?
            .into_iter()
            .find(|f| f.role == "inbox")
            .unwrap();
        let raw=format!("From: {name} <{address}>\r\nTo: {}\r\nSubject: {subject}\r\nMessage-ID: <demo-{i}@example.com>\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body}",a.address);
        let mut m = summary(raw.as_bytes(), &a, &f, i as u32 + 1, 1);
        m.timestamp = now() - (i as i64 * 1720);
        m.unread = i < 4;
        m.starred = i == 0;
        {
            let db = engine.connection()?;
            MailEngine::put_message(&db, &m)?;
        }
        engine.store_body_internal(&parse_body(raw.as_bytes(), &m.id)?)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keeps_safe_mail_styling_but_not_active_content() {
        let html=clean_html("<table role='presentation' style='border:1px solid #ddd;max-width:600px'><tr><td style='color:#333;background-color:#fff;padding:24px;position:fixed;background:url(https://tracker.test/x)'>Hello</td></tr></table><script>bad()</script>");
        assert!(html.contains("padding:24px"));
        assert!(html.contains("background-color:#fff"));
        assert!(html.contains("role=\"presentation\""));
        assert!(!html.contains("position") && !html.contains("script"));
    }
    #[test]
    fn nested_layout_tables_do_not_amplify_content() {
        let mut html = "<p>UNIQUE_BODY_TOKEN</p>".to_string();
        for _ in 0..7 {
            html = format!("<table><tr><td>{html}</td></tr><tr><td>Footer</td></tr></table>");
        }
        let raw = format!("Content-Type: text/html; charset=utf-8\r\n\r\n{html}");
        let body = parse_body(raw.as_bytes(), "nested-tables").unwrap();
        let copies = body
            .markdown
            .replace("\\_", "_")
            .matches("UNIQUE_BODY_TOKEN")
            .count();
        println!(
            "Nested table: input={} output={} copies={}",
            raw.len(),
            body.markdown.len(),
            copies
        );
        assert_eq!(copies, 1);
        assert!(body.markdown.len() < 4096);
    }
    #[test]
    fn image_sources_retained_but_active_content_removed() {
        let s=clean_html("<script>steal()</script><img src='https://tracker.test/x'><p style='background:url(https://bad.test)'>Hello <b>world</b></p><a href='javascript:evil()'>link</a>");
        assert!(!s.contains("script"));
        assert!(s.contains("tracker")); // Fetch policy belongs to the isolated reader's CSP.
        assert!(!s.contains("javascript"));
        assert!(s.contains("background"));
        assert!(s.contains("Hello"));
    }
    #[test]
    fn stylesheet_layout_and_image_only_actions_survive() {
        let raw = b"Content-Type: text/html; charset=utf-8\r\n\r\n<html><head><title>Do not preview this</title><style>.receipt td{padding:12px;color:#123456}@media(max-width:600px){.receipt{width:100%}}</style></head><body><table class='receipt' width='600' cellpadding='0'><tr><td>Total</td><td>20.00</td></tr></table><a name='action' href='https://example.com/verify?token=a%2Bb%3D&amp;next=%2Finbox'><img src='https://example.com/button.png' alt='Verify email' width='140' height='40' onerror='bad()'></a></body></html>";
        let body = parse_body(raw, "image-action").unwrap();
        assert!(body.html.contains("<style>") && body.html.contains("@media"));
        assert!(body.html.contains("class=\"receipt\"") && body.html.contains("width=\"600\""));
        assert!(body
            .html
            .contains("lightmail-image-link-label\">Verify email</span>"));
        assert!(!body.html.contains("onerror") && !body.html.contains("Do not preview"));
        assert!(body
            .markdown
            .contains("[Verify email](https://example.com/verify?token=a%2Bb%3D&next=%2Finbox)"));
        assert!(!body.markdown.contains("@media") && !body.markdown.contains("<a "));
        let legacy = crate::html::restore_link_labels("<a href='https://example.com/verify'></a>");
        assert!(legacy.contains("打开链接") && !legacy.contains(crate::html::RENDER_MARKER));
    }
    #[test]
    fn utf7_names() {
        assert_eq!(decode_folder("&ZeVnLIqe-"), "日本語");
        assert_eq!(decode_folder("A&-B"), "A&B");
    }
    #[test]
    fn mime_decodes_and_keeps_quote() {
        let raw=b"Subject: Test\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nHello=20world\r\n> earlier mail";
        let b = parse_body(raw, "x").unwrap();
        assert!(b.text.contains("Hello world"));
        assert!(b.markdown.contains("> earlier mail"));
    }
    #[test]
    fn mixed_body_preserves_every_section() {
        let raw=b"MIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain\r\n\r\nBefore\r\n--x\r\nContent-Type: text/html\r\n\r\n<p>Middle</p>\r\n--x\r\nContent-Type: text/plain\r\n\r\nAfter\r\n--x--\r\n";
        let body = parse_body(raw, "mixed").unwrap();
        for term in ["Before", "Middle", "After"] {
            assert!(body.markdown.contains(term), "{term}");
        }
    }
    #[test]
    fn alternatives_are_not_duplicated() {
        let raw=b"MIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain\r\n\r\nPlain alternative\r\n--x\r\nContent-Type: text/html\r\n\r\n<p>HTML alternative</p>\r\n--x--\r\n";
        let body = parse_body(raw, "alt").unwrap();
        assert!(body.markdown.contains("HTML alternative"));
        assert!(!body.markdown.contains("Plain alternative"));
    }
    #[test]
    fn plain_text_paragraphs_are_not_treated_as_html() {
        let raw=b"Content-Type: text/plain; charset=utf-8\r\n\r\nHello\n\nFirst paragraph.\n\nSecond paragraph.";
        let b = parse_body(raw, "x").unwrap();
        assert_eq!(b.markdown, "Hello\n\nFirst paragraph.\n\nSecond paragraph.");
        assert!(b.html.is_empty());
    }
}
