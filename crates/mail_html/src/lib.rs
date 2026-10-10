//! The platform-independent contract for an HTML mail reading surface.
//!
//! A renderer belongs to the UI thread. It never receives account credentials,
//! mail store paths, or permission to execute mail content.

/// A decoded MIME image referenced by the HTML body's `cid:` URLs.
#[derive(Clone, Debug)]
pub struct InlineImage {
    pub content_id: String,
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// Network images are an explicit choice for the current message only.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RemoteImagePolicy {
    #[default]
    Block,
    Allow,
}

#[derive(Clone, Debug, Default)]
pub struct HtmlDocument {
    pub html: String,
    pub inline_images: Vec<InlineImage>,
    pub remote_images: RemoteImagePolicy,
}

/// Readiness of the current document, including asynchronous backend failures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HtmlStatus {
    Loading,
    Ready,
    Failed(String),
}

/// A backend-independent reading surface. Native handles belong to its host.
pub trait HtmlRenderer {
    fn render(&self, document: HtmlDocument) -> Result<(), String>;
    fn status(&self) -> HtmlStatus;
    fn has_blocked_images(&self) -> bool;
}
