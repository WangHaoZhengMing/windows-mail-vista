use crate::html::{HtmlDocument, InlineImage, RemoteImagePolicy};
use ammonia::{Builder, UrlRelative};
use base64::{Engine, engine::general_purpose::STANDARD};
use html5ever::{parse_document, serialize, tendril::TendrilSink};
use markup5ever_rcdom::{NodeData, RcDom, SerializableHandle};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use url::Url;

#[path = "css.rs"]
mod css;

pub(super) const ORIGIN: &str = "https://winmail.invalid";
const MAX_HTML_BYTES: usize = 50 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 50 * 1024 * 1024;

pub(super) fn policy(remote: RemoteImagePolicy) -> &'static str {
    match remote {
        RemoteImagePolicy::Block => {
            "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src https://winmail.invalid; font-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; sandbox allow-same-origin"
        }
        RemoteImagePolicy::Allow => {
            "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; img-src https://winmail.invalid http: https:; font-src 'none'; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'; sandbox allow-same-origin"
        }
    }
}

pub(super) struct Resource {
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// WebView2's page and CID images are served from this in-memory origin.
pub(super) struct Document {
    pub uri: String,
    pub blocked_images: bool,
    pub remote_images: RemoteImagePolicy,
    resources: HashMap<String, Rc<Resource>>,
}

/// MSHTML has no CSP support, so this output is independently safe static HTML.
pub(super) struct PreparedHtml {
    pub html: String,
    pub blocked_images: bool,
}

impl Document {
    pub fn prepare(document: &HtmlDocument, generation: u64) -> Result<Self, String> {
        validate(document)?;
        let uri = format!("{ORIGIN}/message/{generation}/index.html");
        let mut resources = HashMap::new();
        let mut cid_urls = HashMap::new();
        for (index, image) in document.inline_images.iter().enumerate() {
            if let Some((id, content_type)) = image_identity(image)
                && !cid_urls.contains_key(&id)
            {
                let image_uri = format!("{ORIGIN}/message/{generation}/cid/{index}");
                cid_urls.insert(id, image_uri.clone());
                resources.insert(
                    image_uri,
                    Rc::new(Resource {
                        content_type,
                        bytes: image.bytes.clone(),
                    }),
                );
            }
        }
        let urls = Arc::new(ImageUrls::new(cid_urls, document.remote_images));
        let sanitized = sanitize(&document.html, &urls)?;
        let page = page(&sanitized, Some(policy(document.remote_images)));
        resources.insert(
            uri.clone(),
            Rc::new(Resource {
                content_type: "text/html; charset=utf-8".into(),
                bytes: page.into_bytes(),
            }),
        );
        Ok(Self {
            uri,
            resources,
            blocked_images: urls.blocked.load(Ordering::Relaxed),
            remote_images: document.remote_images,
        })
    }
    pub fn resource(&self, uri: &str) -> Option<Rc<Resource>> {
        self.resources.get(uri).cloned()
    }
    pub fn is_page(&self, uri: &str) -> bool {
        uri.split('#').next() == Some(self.uri.as_str())
    }
    pub fn allows_remote_image(&self, uri: &str) -> bool {
        self.remote_images == RemoteImagePolicy::Allow && remote_image_url(uri).is_some()
    }
}

pub(super) fn prepare_mshtml(document: &HtmlDocument) -> Result<PreparedHtml, String> {
    validate(document)?;
    let mut cid_urls = HashMap::new();
    for image in &document.inline_images {
        if let Some((id, content_type)) = image_identity(image) {
            cid_urls.entry(id).or_insert_with(|| {
                format!(
                    "data:{content_type};base64,{}",
                    STANDARD.encode(&image.bytes)
                )
            });
        }
    }
    let urls = Arc::new(ImageUrls::new(cid_urls, document.remote_images));
    let sanitized = sanitize(&document.html, &urls)?;
    Ok(PreparedHtml {
        html: page(&sanitized, None),
        blocked_images: urls.blocked.load(Ordering::Relaxed),
    })
}

fn validate(document: &HtmlDocument) -> Result<(), String> {
    if document.html.len() > MAX_HTML_BYTES {
        return Err("HTML body exceeds 50 MiB".into());
    }
    let size = document
        .inline_images
        .iter()
        .try_fold(0usize, |total, image| total.checked_add(image.bytes.len()))
        .ok_or("Inline image size overflow")?;
    if size > MAX_IMAGE_BYTES {
        return Err("Inline images exceed 50 MiB".into());
    }
    Ok(())
}

fn image_identity(image: &InlineImage) -> Option<(String, String)> {
    let content_type = image.content_type.trim().to_ascii_lowercase();
    // SVG and arbitrary data URLs can contain active content. Only MIME raster
    // images supplied by our own decoder become inline resources.
    if !matches!(
        content_type.as_str(),
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/bmp"
            | "image/x-icon"
            | "image/vnd.microsoft.icon"
    ) {
        return None;
    }
    let id = image.content_id.trim().trim_matches(['<', '>']).to_owned();
    (!id.is_empty()).then_some((id, content_type))
}

struct ImageUrls {
    cids: HashMap<String, String>,
    remote: RemoteImagePolicy,
    blocked: AtomicBool,
}
impl ImageUrls {
    fn new(cids: HashMap<String, String>, remote: RemoteImagePolicy) -> Self {
        Self {
            cids,
            remote,
            blocked: AtomicBool::new(false),
        }
    }
    fn resolve(&self, value: &str) -> Option<String> {
        let value = value.trim();
        if value
            .get(..4)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("cid:"))
        {
            return self.cids.get(&decode_cid(&value[4..])).cloned();
        }
        let remote = remote_image_url(value)?;
        match self.remote {
            RemoteImagePolicy::Allow => Some(remote),
            RemoteImagePolicy::Block => {
                self.blocked.store(true, Ordering::Relaxed);
                None
            }
        }
    }
}

fn sanitize(html: &str, urls: &Arc<ImageUrls>) -> Result<String, String> {
    let mut builder = Builder::default();
    let attribute_urls = urls.clone();
    builder
        .rm_clean_content_tags(&["style"])
        .add_tags(&["style", "font", "center"])
        .add_clean_content_tags(&[
            "iframe", "object", "embed", "applet", "svg", "math", "form", "input", "button",
            "textarea", "select", "video", "audio", "template", "noscript",
        ])
        .add_generic_attributes(&[
            "style", "class", "id", "dir", "align", "bgcolor", "width", "height",
        ])
        .add_tag_attributes("font", &["color", "face", "size"])
        .add_tag_attributes(
            "table",
            &["cellpadding", "cellspacing", "border", "background"],
        )
        .add_tag_attributes("td", &["background", "valign", "colspan", "rowspan"])
        .add_tag_attributes("th", &["background", "valign", "colspan", "rowspan"])
        .url_schemes(HashSet::from(["http", "https", "mailto", "cid"]))
        // Image URLs beginning // are resolved as HTTPS by our filter. Other
        // relative image/link paths are rejected by the same explicit filter.
        .url_relative(UrlRelative::PassThrough)
        .attribute_filter(
            move |element, attribute, value| match (element, attribute) {
                ("img", "src") | (_, "background") => attribute_urls.resolve(value).map(Cow::Owned),
                ("a", "href") => external_url(value).map(Cow::Owned),
                (_, "style") => Some(Cow::Owned(css::declarations(value, &attribute_urls))),
                (_, "srcset" | "poster" | "action" | "formaction") => None,
                _ => Some(Cow::Borrowed(value)),
            },
        );
    let sanitized = builder.clean(html).to_string();
    // A real HTML parser handles style elements and encoded HTML attributes.
    // CSS is tokenized separately; CSS escapes cannot disguise url/expression.
    let dom = parse_document(RcDom::default(), Default::default()).one(sanitized);
    let mut pending = vec![dom.document.clone()];
    while let Some(node) = pending.pop() {
        if let NodeData::Element { name, .. } = &node.data
            && name.local.as_ref() == "style"
        {
            for child in node.children.borrow().iter() {
                if let NodeData::Text { contents } = &child.data {
                    let cleaned = css::stylesheet(&contents.borrow(), urls);
                    *contents.borrow_mut() = cleaned.into();
                }
            }
        }
        pending.extend(node.children.borrow().iter().cloned());
    }
    let mut output = Vec::new();
    // Leading style tags are placed in head by the parser. Retain only styles.
    if let Some(head) = find_element(&dom.document, "head") {
        for child in head.children.borrow().iter() {
            if let NodeData::Element { name, .. } = &child.data
                && name.local.as_ref() == "style"
            {
                serialize_node(&mut output, child)?;
            }
        }
    }
    // page() owns the document/head envelope; serialize only body children.
    let body = find_element(&dom.document, "body").ok_or("Sanitized HTML has no body")?;
    for child in body.children.borrow().iter() {
        serialize_node(&mut output, child)?;
    }
    String::from_utf8(output).map_err(|error| error.to_string())
}

fn serialize_node(output: &mut Vec<u8>, node: &markup5ever_rcdom::Handle) -> Result<(), String> {
    serialize(
        output,
        &SerializableHandle::from(node.clone()),
        html5ever::serialize::SerializeOpts {
            traversal_scope: html5ever::serialize::TraversalScope::IncludeNode,
            ..Default::default()
        },
    )
    .map_err(|error| error.to_string())
}

fn find_element(node: &markup5ever_rcdom::Handle, tag: &str) -> Option<markup5ever_rcdom::Handle> {
    let mut pending = vec![node.clone()];
    while let Some(node) = pending.pop() {
        if let NodeData::Element { name, .. } = &node.data
            && name.local.as_ref() == tag
        {
            return Some(node);
        }
        pending.extend(node.children.borrow().iter().cloned());
    }
    None
}

fn page(sanitized: &str, csp: Option<&str>) -> String {
    let csp = csp
        .map(|value| format!("<meta http-equiv=Content-Security-Policy content=\"{value}\">"))
        .unwrap_or_default();
    format!(
        "<!doctype html><html><head><meta charset=utf-8><meta http-equiv=X-UA-Compatible content=IE=edge><meta name=referrer content=no-referrer>{csp}<style>html{{background:#fff;color:#111}}body{{margin:12px;font:14px 'Segoe UI',sans-serif;word-wrap:break-word}}img{{max-width:100%}}table{{max-width:100%}}</style></head><body>{sanitized}</body></html>"
    )
}

fn decode_cid(value: &str) -> String {
    url::form_urlencoded::parse(format!("id={}", value.replace('+', "%2B")).as_bytes())
        .next()
        .map(|(_, value)| value.into_owned())
        .unwrap_or_default()
}
fn remote_image_url(value: &str) -> Option<String> {
    let value = if value.starts_with("//") {
        format!("https:{value}")
    } else {
        value.to_owned()
    };
    let url = external_url(&value)?;
    let parsed = Url::parse(&url).ok()?;
    matches!(parsed.scheme(), "http" | "https").then_some(url)
}
pub(super) fn external_url(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) {
        return None;
    }
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https" | "mailto")
        || url.host_str() == Some("winmail.invalid")
    {
        return None;
    }
    if matches!(url.scheme(), "http" | "https")
        && (url.host_str().is_none() || !url.username().is_empty() || url.password().is_some())
    {
        return None;
    }
    Some(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(html: &str, remote_images: RemoteImagePolicy) -> HtmlDocument {
        HtmlDocument {
            html: html.into(),
            inline_images: vec![InlineImage {
                content_id: "photo@x".into(),
                content_type: "image/png".into(),
                bytes: vec![1, 2, 3],
            }],
            remote_images,
        }
    }
    #[test]
    fn retains_tables_css_cid_and_records_only_blocked_images() {
        let input = document(
            r#"<style>.x{color:red;background:url(https://tracker.example/background)}</style><table cellpadding="4"><tr><td style="color:blue">Hi<img src="cid:photo%40x"><img src="https://tracker.example/p"><img src="file:///secret"><img src="data:image/png;base64,eA=="></td></tr></table>"#,
            RemoteImagePolicy::Block,
        );
        let virtual_doc = Document::prepare(&input, 2).unwrap();
        let resource = virtual_doc.resource(&virtual_doc.uri).unwrap();
        let html = String::from_utf8_lossy(&resource.bytes);
        assert!(html.contains("<table cellpadding=\"4\""));
        assert!(html.contains("color:red"));
        assert!(html.contains("style=\"color:blue"));
        assert!(html.contains("https://winmail.invalid/message/2/cid/0"));
        assert!(virtual_doc.blocked_images);
        assert!(!html.contains("tracker.example"));
        assert!(!html.contains("file:///"));
        assert!(!html.contains("data:image"));
        let mshtml = prepare_mshtml(&input).unwrap();
        assert!(mshtml.blocked_images);
        assert!(mshtml.html.contains("data:image/png;base64,AQID"));
        assert!(!mshtml.html.contains("tracker.example"));
        assert!(
            !prepare_mshtml(&document(
                "<img src=cid:photo%40x>",
                RemoteImagePolicy::Block
            ))
            .unwrap()
            .blocked_images
        );
        assert!(
            !prepare_mshtml(&document("<p>plain</p>", RemoteImagePolicy::Block))
                .unwrap()
                .blocked_images
        );
    }
    #[test]
    fn consent_allows_http_images_but_no_active_content_or_network_css() {
        let input = document(
            r#"<style>@import 'https://evil.example/css';@font-face{font-family:X;src:url(https://evil.example/font)}.x{color:red;background:url(https://pictures.example/background);behavior:url(https://evil.example/htc);width:expression(evil());filter:progid:DXImageTransform.Microsoft.AlphaImageLoader(src='https://evil.example/p')}</style><p style="background: u\72l('https://pictures.example/inline');width:ex\70ression(evil())">Hi</p><img src="https://pictures.example/p"><script>evil()</script><iframe src="https://evil.example">bad</iframe><object data="https://evil.example">bad</object><form>bad</form><svg>bad</svg><a href="javascript:evil()">bad</a><a href="https://example.com">good</a>"#,
            RemoteImagePolicy::Allow,
        );
        let output = prepare_mshtml(&input).unwrap();
        assert!(!output.blocked_images);
        for bad in [
            "evil.example",
            "evil()",
            "expression",
            "behavior",
            "AlphaImageLoader",
            "<iframe",
            "<object",
            "<form",
            "<script",
            "javascript:",
        ] {
            assert!(!output.html.contains(bad), "{bad}");
        }
        assert!(output.html.contains("https://pictures.example/p"));
        assert!(output.html.contains("https://pictures.example/inline"));
        assert!(output.html.contains("https://pictures.example/background"));
        assert!(output.html.contains("href=\"https://example.com/\""));
    }
    #[test]
    fn document_generations_and_link_policy_do_not_cross() {
        let first = Document::prepare(&document("", RemoteImagePolicy::Block), 4).unwrap();
        let next = Document::prepare(&document("", RemoteImagePolicy::Block), 5).unwrap();
        assert!(!next.is_page(&first.uri));
        assert!(next.is_page(&format!("{}#part", next.uri)));
        assert!(
            next.resource(&format!("{ORIGIN}/message/4/cid/0"))
                .is_none()
        );
        for bad in [
            "file:///C:/x",
            "javascript:x",
            "https://winmail.invalid/x",
            "http://a\n",
            "ms-settings:x",
            "https://user:password@host/x",
        ] {
            assert!(external_url(bad).is_none(), "{bad}");
        }
        assert!(external_url("mailto:a@example.com").is_some());
    }

    #[test]
    fn protocol_relative_and_escaped_css_images_follow_the_current_policy() {
        let html = r#"<img src="//pictures.example/photo"><p style="background:u\72l(\63id:photo%40x);color:red">Hi</p><style>@media screen and (max-width:640px){.x{padding:4px;background-image:url('//pictures.example/background')}}.y{font-family:'\3c/style\3e\3cscript\3e';width:e\78pression(evil())}</style>"#;
        let blocked = prepare_mshtml(&document(html, RemoteImagePolicy::Block)).unwrap();
        assert!(blocked.blocked_images);
        assert!(!blocked.html.contains("pictures.example"));
        assert!(blocked.html.contains("data:image/png;base64,AQID"));
        assert!(blocked.html.contains("@media"));
        assert!(blocked.html.contains("padding:4px"));
        assert!(!blocked.html.contains("<script"));
        assert!(!blocked.html.contains("expression"));
        let allowed = prepare_mshtml(&document(html, RemoteImagePolicy::Allow)).unwrap();
        assert!(!allowed.blocked_images);
        assert!(allowed.html.contains("https://pictures.example/photo"));
        assert!(allowed.html.contains("https://pictures.example/background"));
    }
}
