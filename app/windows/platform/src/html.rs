//! Windows HTML hosts, selected behind the shared mail renderer contract.

mod content;
mod mshtml;
mod webview2;

pub use mail_html::{HtmlDocument, HtmlRenderer, HtmlStatus, InlineImage, RemoteImagePolicy};
use windows_sys::Win32::{
    Foundation::HWND,
    UI::WindowsAndMessaging::{SW_HIDE, SW_SHOW, ShowWindow},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowsHtmlEngine {
    #[default]
    Mshtml,
    WebView2,
}

pub(super) trait HostedHtmlRenderer: HtmlRenderer {
    fn hwnd(&self) -> HWND;
}

/// An owned reading surface. MSHTML is the default Windows backend.
pub struct HtmlView {
    renderer: Box<dyn HostedHtmlRenderer>,
}

impl HtmlView {
    /// # Safety
    /// Both handles must be live windows on this UI thread, and remain live
    /// until this view is dropped. `message` must be a handled WM_APP message.
    pub unsafe fn new(parent: HWND, notify: HWND, message: u32) -> Result<Self, String> {
        unsafe { Self::new_with_engine(parent, notify, message, WindowsHtmlEngine::Mshtml) }
    }

    /// # Safety
    /// The same lifetime and UI-thread rules as `new` apply.
    pub unsafe fn new_with_engine(
        parent: HWND,
        notify: HWND,
        message: u32,
        engine: WindowsHtmlEngine,
    ) -> Result<Self, String> {
        let renderer: Box<dyn HostedHtmlRenderer> = match engine {
            WindowsHtmlEngine::Mshtml => {
                Box::new(unsafe { mshtml::MshtmlRenderer::new(parent, notify, message)? })
            }
            WindowsHtmlEngine::WebView2 => {
                Box::new(unsafe { webview2::WebView2Renderer::new(parent, notify, message)? })
            }
        };
        Ok(Self { renderer })
    }

    /// Borrowed native handle; its owner is this view.
    pub fn hwnd(&self) -> HWND {
        self.renderer.hwnd()
    }
    pub fn render(&self, document: HtmlDocument) -> Result<(), String> {
        self.renderer.render(document)
    }
    pub fn status(&self) -> HtmlStatus {
        self.renderer.status()
    }
    pub fn has_blocked_images(&self) -> bool {
        self.renderer.has_blocked_images()
    }

    /// Compatibility helper. Explicit policies use `render`.
    pub fn set_content(&self, html: &str, images: Vec<InlineImage>) -> Result<(), String> {
        self.render(HtmlDocument {
            html: html.into(),
            inline_images: images,
            remote_images: RemoteImagePolicy::Block,
        })
    }

    pub fn set_visible(&self, visible: bool) {
        unsafe {
            ShowWindow(self.hwnd(), if visible { SW_SHOW } else { SW_HIDE });
        }
    }
}

impl HtmlRenderer for HtmlView {
    fn render(&self, document: HtmlDocument) -> Result<(), String> {
        self.renderer.render(document)
    }
    fn status(&self) -> HtmlStatus {
        self.renderer.status()
    }
    fn has_blocked_images(&self) -> bool {
        self.renderer.has_blocked_images()
    }
}
