//! Platform-independent composition and export. Views provide localized date labels.
use crate::{models::*, translation::*};
use std::collections::HashSet;

#[derive(Clone, Copy, uniffi::Enum)]
pub enum ComposeMode {
    New,
    Reply,
    ReplyAll,
    Forward,
}
#[derive(Clone, Copy, uniffi::Enum)]
pub enum ExportMode {
    Original,
    Translated,
    Bilingual,
}

#[uniffi::export]
pub fn compose_draft(
    account: Account,
    message: Option<MessageSummary>,
    body: Option<MailBody>,
    mode: ComposeMode,
    date_label: String,
) -> Draft {
    let mut d = Draft {
        id: uuid::Uuid::new_v4().to_string(),
        account_id: account.id,
        to: String::new(),
        cc: String::new(),
        bcc: String::new(),
        subject: String::new(),
        body: String::new(),
        attachment_paths: vec![],
        reply_to_message_id: String::new(),
        references: String::new(),
        status: "draft".into(),
        last_error: String::new(),
        created_at: now(),
        updated_at: now(),
        send_after: 0,
    };
    let Some(m) = message.filter(|_| !matches!(mode, ComposeMode::New)) else {
        return d;
    };
    if matches!(mode, ComposeMode::Forward) {
        d.subject = if m.subject.to_lowercase().starts_with("fwd:") {
            m.subject.clone()
        } else {
            format!("Fwd: {}", m.subject)
        };
    } else {
        d.to = if m.reply_to_address.is_empty() {
            m.from_address.clone()
        } else {
            m.reply_to_address.clone()
        };
        if matches!(mode, ComposeMode::ReplyAll) {
            let mut seen: HashSet<String> = [
                account.address.to_lowercase(),
                m.from_address.to_lowercase(),
                d.to.to_lowercase(),
            ]
            .into_iter()
            .collect();
            d.cc = format!("{},{}", m.to_addresses, m.cc_addresses)
                .split([',', ';', '\n'])
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .filter(|s| seen.insert(s.to_lowercase()))
                .collect::<Vec<_>>()
                .join(", ");
        }
        d.subject = if m.subject.to_lowercase().starts_with("re:") {
            m.subject.clone()
        } else {
            format!("Re: {}", m.subject)
        };
        d.reply_to_message_id = m.message_id.clone();
        d.references = m.message_id;
    }
    let sender = if m.from_name.is_empty() {
        m.from_address
    } else {
        m.from_name
    };
    let quote = body
        .map(|b| b.text)
        .unwrap_or_default()
        .split('\n')
        .map(|s| format!("> {s}"))
        .collect::<Vec<_>>()
        .join("\n");
    d.body = format!("\n\n{date_label}，{sender} 写道：\n{quote}");
    d
}

#[uniffi::export]
pub fn export_markdown(
    message: MessageSummary,
    body: MailBody,
    translation: Option<TranslationResult>,
    mode: ExportMode,
    date_label: String,
) -> Result<String> {
    if !matches!(mode, ExportMode::Original) {
        let t = translation.as_ref().ok_or_else(|| fail("译文尚未完成"))?;
        validate_translation(&body, t)?;
    }
    let title = if matches!(mode, ExportMode::Original) {
        &message.subject
    } else {
        &translation.as_ref().unwrap().subject
    };
    let sender = if message.from_name.is_empty() {
        &message.from_address
    } else {
        &message.from_name
    };
    let mut output = format!(
        "# {title}\n\n- 发件人：{sender} <{}>\n- 收件人：{}\n- 日期：{date_label}\n\n---\n\n",
        message.from_address, message.to_addresses
    );
    match mode {
        ExportMode::Original => output.push_str(&body.markdown),
        ExportMode::Translated => output.push_str(&translation_markdown(translation.unwrap())),
        ExportMode::Bilingual => {
            let t = translation.unwrap();
            for b in translation_blocks(body.markdown) {
                let target = &t.blocks.iter().find(|p| p.id == b.id).unwrap().text;
                output.push_str(&format!("{}\n\n{target}\n\n---\n\n", b.text));
            }
        }
    }
    if !body.attachments.is_empty() {
        output.push_str("\n\n## 附件\n");
        for a in body.attachments {
            output.push_str(&format!("- {}（{} bytes）\n", a.filename, a.size));
        }
    }
    Ok(output)
}

pub(crate) fn validate_translation(body: &MailBody, result: &TranslationResult) -> Result<()> {
    let source = translation_blocks(body.markdown.clone());
    let ids: HashSet<_> = result.blocks.iter().map(|b| b.id).collect();
    if result.source_hash != body.content_hash
        || result.blocks.len() != source.len()
        || ids != source.iter().map(|b| b.id).collect()
        || result.blocks.iter().any(|b| b.text.trim().is_empty())
    {
        return Err(fail("译文与正文不匹配或存在缺失段落"));
    }
    Ok(())
}
