use crate::{MAX_MESSAGE, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use mailparse::{MailAddr, MailHeaderMap, ParsedMail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub subject: String,
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub reply_to: String,
    pub date: String,
    pub message_id: String,
}
#[derive(Debug)]
pub struct Attachment {
    pub name: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
    pub content_id: Option<String>,
    pub inline: bool,
}

impl Attachment {
    /// A supported MIME image that belongs to the HTML reading surface.
    /// Inline PDFs, SVGs and unnamed binary parts remain downloadable files.
    pub fn is_embedded_image(&self) -> bool {
        self.inline && self.content_id.is_some() && self.is_renderable_image()
    }

    fn is_renderable_image(&self) -> bool {
        matches!(
            self.content_type.trim().to_ascii_lowercase().as_str(),
            "image/png"
                | "image/jpeg"
                | "image/gif"
                | "image/webp"
                | "image/bmp"
                | "image/x-icon"
                | "image/vnd.microsoft.icon"
        )
    }
}
pub struct Message {
    pub summary: Summary,
    pub body: String,
    pub html: Option<String>,
    pub attachments: Vec<Attachment>,
}

impl Message {
    /// MIME image parts addressable by a `cid:` reference in the HTML body.
    pub fn inline_images(&self) -> impl Iterator<Item = &Attachment> {
        self.attachments.iter().filter(|attachment| {
            attachment.content_id.is_some() && attachment.is_renderable_image()
        })
    }

    /// Files shown by the attachment UI, excluding supported embedded images.
    pub fn downloadable_attachments(&self) -> impl Iterator<Item = &Attachment> {
        self.attachments
            .iter()
            .filter(|attachment| !attachment.is_embedded_image())
    }
}

pub fn parse(raw: &[u8]) -> Result<Message> {
    if raw.len() > MAX_MESSAGE {
        return Err("Message exceeds 50 MiB".into());
    }
    let mail = mailparse::parse_mail(raw).map_err(|e| e.to_string())?;
    let get = |name| mail.headers.get_first_value(name).unwrap_or_default();
    let summary = Summary {
        subject: get("Subject"),
        from: get("From"),
        to: get("To"),
        cc: get("Cc"),
        reply_to: get("Reply-To"),
        date: get("Date"),
        message_id: get("Message-ID"),
    };
    let mut plain = Vec::new();
    let mut html = Vec::new();
    let mut attachments = Vec::new();
    collect(&mail, &mut plain, &mut html, &mut attachments, 0)?;
    let html = (!html.is_empty()).then(|| html.join("\n\n"));
    let body = if !plain.is_empty() {
        plain.join("\n\n")
    } else {
        html.as_deref().map(html_text).unwrap_or_default()
    };
    Ok(Message {
        summary,
        body,
        html,
        attachments,
    })
}
fn collect(
    mail: &ParsedMail<'_>,
    plain: &mut Vec<String>,
    html: &mut Vec<String>,
    attachments: &mut Vec<Attachment>,
    depth: usize,
) -> Result<()> {
    if depth > 32 {
        return Err("MIME nesting exceeds 32 levels".into());
    }
    let disposition = mail.get_content_disposition();
    let filename = disposition
        .params
        .get("filename")
        .or_else(|| mail.ctype.params.get("name"));
    let content_id = mail
        .headers
        .get_first_value("Content-ID")
        .map(|id| {
            id.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .trim()
                .to_owned()
        })
        .filter(|id| !id.is_empty());
    let is_attachment = disposition.disposition == mailparse::DispositionType::Attachment;
    let cid_binary = content_id.is_some()
        && !matches!(mail.ctype.mimetype.as_str(), "text/plain" | "text/html")
        && mail.subparts.is_empty();
    if filename.is_some() || is_attachment || cid_binary {
        let name = filename
            .map(|s| safe_filename(s))
            .unwrap_or_else(|| format!("attachment-{}.bin", attachments.len() + 1));
        attachments.push(Attachment {
            name,
            content_type: mail.ctype.mimetype.clone(),
            bytes: mail.get_body_raw().map_err(|e| e.to_string())?,
            inline: !is_attachment
                && (content_id.is_some()
                    || disposition.disposition == mailparse::DispositionType::Inline),
            content_id,
        });
    } else if mail.subparts.is_empty() {
        let body = mail.get_body().map_err(|e| e.to_string())?;
        match mail.ctype.mimetype.as_str() {
            "text/plain" => plain.push(body),
            "text/html" => html.push(body),
            _ => (),
        }
    } else {
        for part in &mail.subparts {
            collect(part, plain, html, attachments, depth + 1)?;
        }
    }
    Ok(())
}
fn html_text(html: &str) -> String {
    use html5ever::{parse_document, tendril::TendrilSink};
    use markup5ever_rcdom::{NodeData, RcDom};

    let dom = parse_document(RcDom::default(), Default::default()).one(html);
    let mut out = String::new();
    // Traverse without recursion so deeply nested HTML cannot overflow our stack.
    let mut pending = vec![(dom.document.clone(), false)];
    while let Some((node, ending)) = pending.pop() {
        if ending {
            out.push('\n');
            continue;
        }
        let mut block = false;
        match &node.data {
            NodeData::Text { contents } => out.push_str(&contents.borrow()),
            NodeData::Element { name, attrs, .. } => {
                let tag = name.local.as_ref();
                if matches!(tag, "head" | "script" | "style" | "template" | "noscript") {
                    continue;
                }
                if tag == "img"
                    && let Some(alt) = attrs
                        .borrow()
                        .iter()
                        .find(|attr| attr.name.local.as_ref() == "alt")
                {
                    out.push_str(&alt.value);
                }
                block = matches!(
                    tag,
                    "p" | "div"
                        | "br"
                        | "li"
                        | "tr"
                        | "blockquote"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "pre"
                        | "hr"
                );
                if block {
                    out.push('\n');
                } else if matches!(tag, "td" | "th") {
                    out.push('\t');
                }
            }
            _ => (),
        }
        if block {
            pending.push((node.clone(), true));
        }
        pending.extend(
            node.children
                .borrow()
                .iter()
                .rev()
                .cloned()
                .map(|child| (child, false)),
        );
    }
    out.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}
pub fn safe_filename(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(100)
        .collect();
    let clean = clean.trim_matches([' ', '.']);
    if clean.is_empty() {
        "attachment.bin".into()
    } else {
        format!("attachment-{clean}")
    }
}
pub fn addresses(text: &str) -> Result<Vec<String>> {
    if text.contains(['\r', '\n', '\0']) {
        return Err("Address contains control characters".into());
    }
    let parsed = mailparse::addrparse(text).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for addr in parsed.iter() {
        match addr {
            MailAddr::Single(s) => out.push(s.addr.clone()),
            MailAddr::Group(g) => out.extend(g.addrs.iter().map(|s| s.addr.clone())),
        }
    }
    if out.is_empty()
        || out.iter().any(|a| {
            !a.is_ascii()
                || !a.contains('@')
                || a.contains(['<', '>'])
                || a.bytes()
                    .any(|b| b.is_ascii_whitespace() || b.is_ascii_control())
        })
    {
        return Err("Expected an ASCII email address (SMTPUTF8 is not implemented)".into());
    }
    Ok(out)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Draft {
    pub to: String,
    #[serde(default)]
    pub cc: String,
    #[serde(default)]
    pub bcc: String,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub attachments: Vec<PathBuf>,
}
impl Draft {
    pub fn reply(mail: &Message) -> Self {
        Self {
            to: if mail.summary.reply_to.is_empty() {
                mail.summary.from.clone()
            } else {
                mail.summary.reply_to.clone()
            },
            subject: if mail.summary.subject.to_ascii_lowercase().starts_with("re:") {
                mail.summary.subject.clone()
            } else {
                format!("Re: {}", mail.summary.subject)
            },
            body: format!(
                "\r\n\r\n{} wrote:\r\n{}",
                mail.summary.from,
                mail.body
                    .lines()
                    .map(|line| format!("> {line}"))
                    .collect::<Vec<_>>()
                    .join("\r\n")
            ),
            ..Self::default()
        }
    }
    pub fn mailto(uri: &str) -> Result<Self> {
        let u = url::Url::parse(uri).map_err(|e| e.to_string())?;
        if u.scheme() != "mailto" {
            return Err("Expected mailto URL".into());
        }
        let to =
            url::form_urlencoded::parse(format!("to={}", u.path().replace('+', "%2B")).as_bytes())
                .next()
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default();
        let mut draft = Self {
            to,
            ..Self::default()
        };
        for (k, v) in u.query_pairs() {
            match k.to_ascii_lowercase().as_str() {
                "to" => {
                    if !draft.to.is_empty() {
                        draft.to.push(',');
                    }
                    draft.to.push_str(&v);
                }
                "cc" => draft.cc = v.into_owned(),
                "bcc" => draft.bcc = v.into_owned(),
                "subject" => draft.subject = v.into_owned(),
                "body" => draft.body = v.into_owned(),
                _ => (),
            }
        }
        Ok(draft)
    }
    pub fn recipients(&self) -> Result<Vec<String>> {
        let mut result = addresses(&self.to)?;
        for value in [&self.cc, &self.bcc] {
            if !value.is_empty() {
                result.extend(addresses(value)?);
            }
        }
        result.sort();
        result.dedup();
        Ok(result)
    }
    pub fn build(&self, sender: &str) -> Result<Vec<u8>> {
        self.build_internal(sender, true)
    }
    pub fn build_draft(&self, sender: &str) -> Result<Vec<u8>> {
        self.build_internal(sender, false)
    }
    fn build_internal(&self, sender: &str, sending: bool) -> Result<Vec<u8>> {
        let sender = addresses(sender)?;
        if sender.len() != 1 {
            return Err("Expected one sender address".into());
        }
        if sending {
            self.recipients()?;
        }
        if self.to.contains(['\r', '\n', '\0']) || self.cc.contains(['\r', '\n', '\0']) {
            return Err("Address header contains control characters".into());
        }
        if self.body.len() > MAX_MESSAGE {
            return Err("Message body exceeds 50 MiB".into());
        }
        if self.subject.contains(['\r', '\n', '\0']) {
            return Err("Subject contains header injection".into());
        }
        let date = chrono::Utc::now().to_rfc2822();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let id = format!("{stamp:x}.{}@winmail-rust.local", std::process::id());
        let to = if sending {
            addresses(&self.to)?.join(", ")
        } else {
            encoded_header(&self.to)
        };
        if to.len() > 900 || self.subject.len() > 900 {
            return Err("Header is too long".into());
        }
        let mut raw = format!("From: {}\r\nTo: {}\r\n", sender[0], to);
        if !self.cc.is_empty() {
            let cc = if sending {
                addresses(&self.cc)?.join(", ")
            } else {
                encoded_header(&self.cc)
            };
            if cc.len() > 900 {
                return Err("Cc header is too long".into());
            }
            raw.push_str(&format!("Cc: {cc}\r\n"));
        }
        // Bcc is envelope-only, never serialized to recipients.
        raw.push_str(&format!(
            "Subject: {}\r\nDate: {date}\r\nMessage-ID: <{id}>\r\nMIME-Version: 1.0\r\n",
            encoded_header(&self.subject)
        ));
        let body = base64_lines(self.body.as_bytes());
        if self.attachments.is_empty() {
            raw.push_str(&format!("Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{body}"));
        } else {
            let boundary = format!("winmail-{stamp:x}");
            raw.push_str(&format!("Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n--{boundary}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64\r\n\r\n{body}"));
            for path in &self.attachments {
                let bytes = crate::store::read_bounded(path)?;
                if bytes.len() > MAX_MESSAGE {
                    return Err("Attachment exceeds 50 MiB".into());
                }
                let name = safe_filename(&path.file_name().unwrap_or_default().to_string_lossy());
                let encoded_name: String =
                    url::form_urlencoded::byte_serialize(name.as_bytes()).collect();
                raw.push_str(&format!("--{boundary}\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename*=utf-8''{}\r\nContent-Transfer-Encoding: base64\r\n\r\n{}", encoded_name.replace('+',"%20"), base64_lines(&bytes)));
                if raw.len() > MAX_MESSAGE {
                    return Err("Composed message exceeds 50 MiB".into());
                }
            }
            raw.push_str(&format!("--{boundary}--\r\n"));
        }
        if raw.len() > MAX_MESSAGE {
            return Err("Composed message exceeds 50 MiB".into());
        }
        Ok(raw.into_bytes())
    }
}
fn encoded_header(text: &str) -> String {
    if text.is_ascii() {
        return text.to_owned();
    }
    let mut words = Vec::new();
    let mut chunk = String::new();
    for c in text.chars() {
        if chunk.len() + c.len_utf8() > 42 {
            words.push(format!("=?UTF-8?B?{}?=", STANDARD.encode(chunk.as_bytes())));
            chunk.clear();
        }
        chunk.push(c);
    }
    if !chunk.is_empty() {
        words.push(format!("=?UTF-8?B?{}?=", STANDARD.encode(chunk.as_bytes())));
    }
    words.join("\r\n ")
}
fn base64_lines(bytes: &[u8]) -> String {
    let encoded = STANDARD.encode(bytes);
    let mut out = String::new();
    for line in encoded.as_bytes().chunks(76) {
        out.push_str(std::str::from_utf8(line).unwrap());
        out.push_str("\r\n");
    }
    if out.is_empty() {
        out.push_str("\r\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alternative_retains_html_and_prefers_plain_for_replies() {
        let raw = b"Content-Type: multipart/alternative; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nPlain reply text\r\n--x\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<table><tr><td>HTML text</td></tr></table>\r\n--x--\r\n";
        let mail = parse(raw).unwrap();
        assert_eq!(mail.body.trim(), "Plain reply text");
        assert!(mail.html.unwrap().contains("<table>"));
    }

    #[test]
    fn related_images_without_filenames_keep_content_ids_and_decoded_bytes() {
        let raw = b"Content-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html; charset=utf-8\r\nContent-ID: <body>\r\n\r\n<p>Hello<img src=\"cid:photo@example\" alt=\"Photo\"></p>\r\n--x\r\nContent-Type: image/png\r\nContent-ID: <photo@example>\r\nContent-Transfer-Encoding: base64\r\n\r\nAQIDBA==\r\n--x\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=doc.pdf\r\n\r\nPDF\r\n--x--\r\n";
        let mail = parse(raw).unwrap();
        assert!(mail.html.as_ref().unwrap().contains("cid:photo@example"));
        assert!(mail.body.contains("HelloPhoto"));
        let image = mail.inline_images().next().unwrap();
        assert_eq!(image.content_id.as_deref(), Some("photo@example"));
        assert_eq!(image.bytes, [1, 2, 3, 4]);
        assert!(image.inline);
        assert!(!mail.attachments[1].inline);
    }

    #[test]
    fn html_only_fallback_decodes_entities_and_excludes_active_content() {
        let raw = b"Content-Type: text/html; charset=utf-8\r\n\r\n<head><title>Hidden</title><style>bad css</style></head><p>Fish &amp; chips &lt;3</p><script>bad script</script><div>Second&nbsp;line</div>";
        let mail = parse(raw).unwrap();
        assert_eq!(mail.body, "Fish & chips <3\nSecond line");
        assert!(mail.html.unwrap().contains("bad script"));
    }

    #[test]
    fn inline_documents_remain_downloadable_while_cid_raster_images_render() {
        let raw = b"Content-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<img src=cid:photo>\r\n--x\r\nContent-Type: image/png\r\nContent-ID: <photo>\r\nContent-Disposition: inline\r\n\r\nPNG\r\n--x\r\nContent-Type: application/pdf\r\nContent-ID: <document>\r\nContent-Disposition: inline\r\n\r\nPDF\r\n--x\r\nContent-Type: image/svg+xml\r\nContent-ID: <vector>\r\nContent-Disposition: inline\r\n\r\n<svg/>\r\n--x--\r\n";
        let mail = parse(raw).unwrap();
        assert_eq!(mail.inline_images().count(), 1);
        assert!(mail.attachments[0].is_embedded_image());
        let files = mail
            .downloadable_attachments()
            .map(|part| part.content_type.as_str())
            .collect::<Vec<_>>();
        assert_eq!(files, ["application/pdf", "image/svg+xml"]);
    }
}
