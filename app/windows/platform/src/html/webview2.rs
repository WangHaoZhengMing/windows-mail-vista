use super::{
    HostedHtmlRenderer,
    content::{Document, external_url, policy},
};
use crate::{
    default_data_dir,
    html::{HtmlDocument, HtmlRenderer, HtmlStatus, RemoteImagePolicy},
    strings::wide,
};
use std::result::Result;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    path::Path,
    ptr::null_mut,
    rc::{Rc, Weak},
    sync::OnceLock,
};
use webview2_com::{Microsoft::Web::WebView2::Win32::*, *};
use windows::{
    Win32::{
        Foundation::{HWND as ComHwnd, RECT as ComRect},
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        UI::Shell::SHCreateMemStream,
    },
    core::{BOOL, Interface, PCWSTR, PWSTR},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{Shell::ShellExecuteW, WindowsAndMessaging::*},
};

const CLASS: &str = "WinMailRustHtmlPreview";
const OPEN_LINK: u32 = WM_APP + 113;

/// An owned child window and asynchronous WebView2 controller.
///
/// Keep this value on the UI thread and drop it before destroying its parent.
/// Initialization and navigation finish through the ordinary Windows message
/// loop; status changes post the message supplied to `new`.
pub struct WebView2Renderer {
    shared: Rc<Shared>,
    _apartment: Apartment,
}

impl WebView2Renderer {
    /// Create a hidden preview host. A missing runtime is reported by `status`.
    ///
    /// # Safety
    /// `parent` and `notify` must be live windows on the calling UI thread and
    /// remain live until this view is dropped. `message` must be an application
    /// message understood by `notify`. The host handle returned by `hwnd` is
    /// borrowed and must not be independently destroyed.
    pub unsafe fn new(parent: HWND, notify: HWND, message: u32) -> Result<Self, String> {
        unsafe {
            Self::with_profile(
                parent,
                notify,
                message,
                &default_data_dir().join("html-preview"),
            )
        }
    }

    unsafe fn with_profile(
        parent: HWND,
        notify: HWND,
        message: u32,
        profile: &Path,
    ) -> Result<Self, String> {
        unsafe {
            let thread = GetCurrentThreadId();
            if parent.is_null()
                || notify.is_null()
                || GetWindowThreadProcessId(parent, null_mut()) != thread
                || GetWindowThreadProcessId(notify, null_mut()) != thread
                || message < WM_APP
            {
                return Err(
                    "HTML preview requires live windows on the UI thread and a WM_APP message"
                        .into(),
                );
            }
            let apartment = Apartment::new()?;
            register_class()?;
            let shared = Rc::new(Shared {
                host: Cell::new(null_mut()),
                notify,
                message,
                disposed: Cell::new(false),
                attached: Cell::new(false),
                inner: RefCell::new(Inner {
                    status: HtmlStatus::Loading,
                    runtime_error: None,
                    generation: 0,
                    navigation: None,
                    #[cfg(test)]
                    blocked_navigations: 0,
                    document: Document::prepare(&HtmlDocument::default(), 0)?,
                    environment: None,
                    controller: None,
                    webview: None,
                    links: VecDeque::new(),
                }),
            });
            let userdata = Box::into_raw(Box::new(Rc::downgrade(&shared)));
            let host = CreateWindowExW(
                0,
                wide(CLASS).as_ptr(),
                wide("").as_ptr(),
                WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                0,
                0,
                parent,
                null_mut(),
                GetModuleHandleW(null_mut()),
                userdata.cast(),
            );
            if host.is_null() {
                if !shared.attached.get() {
                    drop(Box::from_raw(userdata));
                }
                return Err(format!(
                    "Cannot create HTML preview window: {}",
                    std::io::Error::last_os_error()
                ));
            }
            shared.host.set(host);
            let view = Self {
                shared,
                _apartment: apartment,
            };
            if let Err(error) = begin_environment(&view.shared, profile) {
                view.shared.runtime_failed(error);
            }
            Ok(view)
        }
    }

    /// Replace the current message. Content is queued while the runtime loads.
    /// Empty HTML clears the previous message without recreating the browser.
    fn replace_document(&self, document: HtmlDocument) -> Result<(), String> {
        if self.shared.disposed.get() {
            return Err("HTML preview has been closed".into());
        }
        if let Some(error) = self.shared.inner.borrow().runtime_error.clone() {
            return Err(error);
        }
        let generation = self
            .shared
            .inner
            .borrow()
            .generation
            .checked_add(1)
            .ok_or("HTML preview generation overflow")?;
        let document = match Document::prepare(&document, generation) {
            Ok(document) => document,
            Err(error) => {
                self.shared.fail(error.clone());
                return Err(error);
            }
        };
        {
            let mut inner = self.shared.inner.borrow_mut();
            inner.document = document;
            inner.generation = generation;
            inner.navigation = None;
        }
        self.shared.set_status(HtmlStatus::Loading);
        match self.shared.navigate() {
            Ok(()) => Ok(()),
            Err(error) => {
                let error = error.to_string();
                self.shared.fail(error.clone());
                Err(error)
            }
        }
    }
}

impl HtmlRenderer for WebView2Renderer {
    fn render(&self, document: HtmlDocument) -> Result<(), String> {
        self.replace_document(document)
    }
    fn status(&self) -> HtmlStatus {
        self.shared.inner.borrow().status.clone()
    }
    fn has_blocked_images(&self) -> bool {
        self.shared.inner.borrow().document.blocked_images
    }
}

impl HostedHtmlRenderer for WebView2Renderer {
    fn hwnd(&self) -> HWND {
        self.shared.host.get()
    }
}

impl Drop for WebView2Renderer {
    fn drop(&mut self) {
        self.shared.shutdown();
        let host = self.shared.host.replace(null_mut());
        if !host.is_null() {
            unsafe {
                DestroyWindow(host);
            }
        }
    }
}

struct Apartment;
impl Apartment {
    fn new() -> Result<Self, String> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|error| error.to_string())?;
        }
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

struct Inner {
    status: HtmlStatus,
    runtime_error: Option<String>,
    generation: u64,
    navigation: Option<(u64, u64)>,
    #[cfg(test)]
    blocked_navigations: u64,
    document: Document,
    environment: Option<ICoreWebView2Environment>,
    controller: Option<ICoreWebView2Controller>,
    webview: Option<ICoreWebView2>,
    links: VecDeque<String>,
}

struct Shared {
    host: Cell<HWND>,
    notify: HWND,
    message: u32,
    disposed: Cell<bool>,
    attached: Cell<bool>,
    inner: RefCell<Inner>,
}

impl Shared {
    fn set_status(&self, status: HtmlStatus) {
        if self.disposed.get() {
            return;
        }
        let changed = {
            let mut inner = self.inner.borrow_mut();
            if inner.status == status {
                false
            } else {
                inner.status = status;
                true
            }
        };
        if changed {
            unsafe {
                PostMessageW(self.notify, self.message, 0, 0);
            }
        }
    }
    fn fail(&self, error: impl ToString) {
        self.set_status(HtmlStatus::Failed(error.to_string()));
    }
    fn runtime_failed(&self, error: impl ToString) {
        let error = error.to_string();
        self.inner.borrow_mut().runtime_error = Some(error.clone());
        self.fail(error);
        self.shutdown_controller();
    }
    fn navigate(&self) -> windows::core::Result<()> {
        let (webview, uri) = {
            let inner = self.inner.borrow();
            (inner.webview.clone(), inner.document.uri.clone())
        };
        if let Some(webview) = webview {
            unsafe {
                webview.Navigate(PCWSTR(wide(&uri).as_ptr()))?;
            }
        }
        Ok(())
    }
    fn resize(&self) {
        let controller = self.inner.borrow().controller.clone();
        if let Some(controller) = controller {
            unsafe {
                let mut rect = std::mem::zeroed();
                if GetClientRect(self.host.get(), &mut rect) != 0
                    && let Err(error) = controller.SetBounds(ComRect {
                        left: rect.left,
                        top: rect.top,
                        right: rect.right,
                        bottom: rect.bottom,
                    })
                {
                    self.fail(error);
                }
            }
        }
    }
    fn queue_link(&self, uri: &str, user_initiated: bool) {
        if !user_initiated || self.disposed.get() {
            return;
        }
        if let Some(uri) = external_url(uri) {
            self.inner.borrow_mut().links.push_back(uri);
            unsafe {
                PostMessageW(self.host.get(), OPEN_LINK, 0, 0);
            }
        }
    }
    fn shutdown(&self) {
        if self.disposed.replace(true) {
            return;
        }
        // Release RefCell before COM calls: closing can dispatch callbacks.
        let controller = {
            let mut inner = self.inner.borrow_mut();
            inner.links.clear();
            inner.webview.take();
            inner.environment.take();
            inner.controller.take()
        };
        if let Some(controller) = controller {
            unsafe {
                let _ = controller.Close();
            }
        }
    }
}

fn register_class() -> Result<(), String> {
    static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
    REGISTERED
        .get_or_init(|| unsafe {
            let mut class: WNDCLASSW = std::mem::zeroed();
            class.lpfnWndProc = Some(host_proc);
            class.hInstance = GetModuleHandleW(null_mut());
            let name = wide(CLASS);
            class.lpszClassName = name.as_ptr();
            if RegisterClassW(&class) == 0 {
                Err(format!(
                    "Cannot register HTML preview window: {}",
                    std::io::Error::last_os_error()
                ))
            } else {
                Ok(())
            }
        })
        .clone()
}

unsafe extern "system" fn host_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lp as *const CREATESTRUCTW);
            let weak = create.lpCreateParams as *mut Weak<Shared>;
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, weak as _);
            if let Some(shared) = (*weak).upgrade() {
                shared.attached.set(true);
                shared.host.set(hwnd);
            }
            return 1;
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Weak<Shared>;
        if !pointer.is_null() {
            if let Some(shared) = (*pointer).upgrade() {
                match message {
                    WM_SIZE => {
                        shared.resize();
                        return 0;
                    }
                    WM_SHOWWINDOW => {
                        let controller = shared.inner.borrow().controller.clone();
                        if let Some(controller) = controller
                            && let Err(error) = controller.SetIsVisible(wp != 0)
                        {
                            shared.fail(error);
                        }
                    }
                    OPEN_LINK => {
                        let uri = shared.inner.borrow_mut().links.pop_front();
                        if let Some(uri) = uri {
                            ShellExecuteW(
                                hwnd,
                                wide("open").as_ptr(),
                                wide(&uri).as_ptr(),
                                null_mut(),
                                null_mut(),
                                SW_SHOWNORMAL,
                            );
                        }
                        return 0;
                    }
                    WM_NCDESTROY => {
                        shared.shutdown();
                        shared.host.set(null_mut());
                    }
                    _ => (),
                }
            }
            if message == WM_NCDESTROY {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                drop(Box::from_raw(pointer));
            }
        }
        DefWindowProcW(hwnd, message, wp, lp)
    }
}

fn live(weak: &Weak<Shared>) -> Option<Rc<Shared>> {
    weak.upgrade().filter(|shared| !shared.disposed.get())
}

unsafe fn begin_environment(shared: &Rc<Shared>, profile: &Path) -> Result<(), String> {
    unsafe {
        std::fs::create_dir_all(profile)
            .map_err(|error| format!("Cannot prepare HTML browser profile: {error}"))?;
        let profile: Vec<u16> = profile.as_os_str().encode_wide().chain(Some(0)).collect();
        let weak = Rc::downgrade(shared);
        let callback = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(
            move |result, environment| {
                if let Some(shared) = live(&weak) {
                    if let Err(error) = result {
                        shared.runtime_failed(format!(
                            "Cannot create HTML browser environment: {error}"
                        ));
                        return Ok(());
                    }
                    let outcome = result.and_then(|()| {
                        let environment =
                            environment.ok_or_else(windows::core::Error::from_thread)?;
                        start_controller(&shared, environment)
                    });
                    if let Err(error) = outcome {
                        shared.runtime_failed(format!(
                            "Cannot start HTML browser controller: {error}"
                        ));
                    }
                }
                Ok(())
            },
        ));
        let options: ICoreWebView2EnvironmentOptions =
            CoreWebView2EnvironmentOptions::default().into();
        CreateCoreWebView2EnvironmentWithOptions(
            PCWSTR::null(),
            PCWSTR(profile.as_ptr()),
            &options,
            &callback,
        )
        .map_err(|error| format!("Cannot request HTML browser environment: {error}"))
    }
}

use std::os::windows::ffi::OsStrExt;

unsafe fn start_controller(
    shared: &Rc<Shared>,
    environment: ICoreWebView2Environment,
) -> windows::core::Result<()> {
    unsafe {
        shared.inner.borrow_mut().environment = Some(environment.clone());
        let weak = Rc::downgrade(shared);
        let callback = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(
            move |result, controller| {
                if let Some(shared) = live(&weak) {
                    if let Err(error) = result {
                        shared.runtime_failed(format!(
                            "Cannot create HTML browser controller: {error}"
                        ));
                        return Ok(());
                    }
                    let outcome = result.and_then(|()| {
                        let controller =
                            controller.ok_or_else(windows::core::Error::from_thread)?;
                        configure(&shared, controller)
                    });
                    if let Err(error) = outcome {
                        shared.runtime_failed(format!(
                            "Cannot configure HTML browser controller: {error}"
                        ));
                    }
                } else if let Some(controller) = controller {
                    let _ = controller.Close();
                }
                Ok(())
            },
        ));
        environment.CreateCoreWebView2Controller(ComHwnd(shared.host.get()), &callback)
    }
}

impl Shared {
    fn shutdown_controller(&self) {
        let controller = {
            let mut inner = self.inner.borrow_mut();
            inner.webview.take();
            inner.controller.take()
        };
        if let Some(controller) = controller {
            unsafe {
                let _ = controller.Close();
            }
        }
    }
}

unsafe fn configure(
    shared: &Rc<Shared>,
    controller: ICoreWebView2Controller,
) -> windows::core::Result<()> {
    unsafe {
        shared.inner.borrow_mut().controller = Some(controller.clone());
        let webview = controller.CoreWebView2()?;
        let settings = webview.Settings()?;
        settings.SetIsScriptEnabled(false)?;
        settings.SetAreHostObjectsAllowed(false)?;
        settings.SetIsWebMessageEnabled(false)?;
        settings.SetAreDefaultScriptDialogsEnabled(false)?;
        settings.SetAreDefaultContextMenusEnabled(true)?;
        settings.SetAreDevToolsEnabled(false)?;
        settings.SetIsStatusBarEnabled(false)?;
        register_events(shared, &webview)?;
        webview.AddWebResourceRequestedFilter(
            PCWSTR(wide("*").as_ptr()),
            COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
        )?;
        shared.inner.borrow_mut().webview = Some(webview);
        shared.resize();
        controller.SetIsVisible(IsWindowVisible(shared.host.get()) != 0)?;
        shared.navigate()
    }
}

unsafe fn register_events(
    shared: &Rc<Shared>,
    webview: &ICoreWebView2,
) -> windows::core::Result<()> {
    unsafe {
        let mut token = 0;
        let weak = Rc::downgrade(shared);
        webview.add_NavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    args.SetCancel(true)?;
                    if let Some(shared) = live(&weak) {
                        let outcome = (|| {
                            let mut uri = PWSTR::null();
                            args.Uri(&mut uri)?;
                            let uri = take_pwstr(uri);
                            let is_page = shared.inner.borrow().document.is_page(&uri);
                            if is_page {
                                let mut id = 0;
                                args.NavigationId(&mut id)?;
                                {
                                    let mut inner = shared.inner.borrow_mut();
                                    inner.navigation = Some((inner.generation, id));
                                }
                                args.SetCancel(false)?;
                            } else {
                                #[cfg(test)]
                                {
                                    shared.inner.borrow_mut().blocked_navigations += 1;
                                }
                                let mut user = BOOL::default();
                                args.IsUserInitiated(&mut user)?;
                                shared.queue_link(&uri, user.as_bool());
                            }
                            Ok::<_, windows::core::Error>(())
                        })();
                        if let Err(error) = outcome {
                            shared.fail(error);
                        }
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
        webview.add_FrameNavigationStarting(
            &NavigationStartingEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetCancel(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        let weak = Rc::downgrade(shared);
        webview.add_NavigationCompleted(
            &NavigationCompletedEventHandler::create(Box::new(move |_, args| {
                if let (Some(shared), Some(args)) = (live(&weak), args) {
                    let outcome = (|| {
                        let mut id = 0;
                        args.NavigationId(&mut id)?;
                        let current = {
                            let inner = shared.inner.borrow();
                            inner.navigation == Some((inner.generation, id))
                        };
                        if current {
                            let mut success = BOOL::default();
                            args.IsSuccess(&mut success)?;
                            if success.as_bool() {
                                shared.set_status(HtmlStatus::Ready);
                            } else {
                                let mut code = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
                                args.WebErrorStatus(&mut code)?;
                                shared.fail(format!("HTML navigation failed ({})", code.0));
                            }
                        }
                        Ok::<_, windows::core::Error>(())
                    })();
                    if let Err(error) = outcome {
                        shared.fail(error);
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
        let weak = Rc::downgrade(shared);
        webview.add_WebResourceRequested(
            &WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
                if let (Some(shared), Some(args)) = (live(&weak), args) {
                    let outcome = respond(&shared, &args);
                    if let Err(error) = outcome {
                        shared.runtime_failed(error);
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
        let weak = Rc::downgrade(shared);
        webview.add_NewWindowRequested(
            &NewWindowRequestedEventHandler::create(Box::new(move |_, args| {
                if let Some(args) = args {
                    args.SetHandled(true)?;
                    if let Some(shared) = live(&weak) {
                        let mut uri = PWSTR::null();
                        args.Uri(&mut uri)?;
                        let uri = take_pwstr(uri);
                        let mut user = BOOL::default();
                        args.IsUserInitiated(&mut user)?;
                        shared.queue_link(&uri, user.as_bool());
                    }
                }
                Ok(())
            })),
            &mut token,
        )?;
        webview.add_PermissionRequested(
            &PermissionRequestedEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        let weak = Rc::downgrade(shared);
        webview.add_ProcessFailed(
            &ProcessFailedEventHandler::create(Box::new(move |_, _| {
                if let Some(shared) = live(&weak) {
                    shared.runtime_failed("HTML renderer process failed");
                }
                Ok(())
            })),
            &mut token,
        )?;
        let webview4: ICoreWebView2_4 = webview.cast()?;
        webview4.add_DownloadStarting(
            &DownloadStartingEventHandler::create(Box::new(|_, args| {
                if let Some(args) = args {
                    args.SetCancel(true)?;
                    args.SetHandled(true)?;
                }
                Ok(())
            })),
            &mut token,
        )?;
        Ok(())
    }
}

unsafe fn respond(
    shared: &Shared,
    args: &ICoreWebView2WebResourceRequestedEventArgs,
) -> windows::core::Result<()> {
    unsafe {
        let environment = shared
            .inner
            .borrow()
            .environment
            .clone()
            .ok_or_else(windows::core::Error::from_thread)?;
        // Install a denial first; errors must never let a request reach the network.
        let deny = response(
            &environment,
            403,
            "text/plain; charset=utf-8",
            b"Blocked",
            RemoteImagePolicy::Block,
        )?;
        args.SetResponse(&deny)?;
        let request = args.Request()?;
        let mut uri = PWSTR::null();
        request.Uri(&mut uri)?;
        let uri = take_pwstr(uri);
        let mut context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL;
        args.ResourceContext(&mut context)?;
        if context == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE
            && shared.inner.borrow().document.allows_remote_image(&uri)
        {
            // Consent permits image requests only. Scripts, CSS imports, fonts,
            // fetch, frames and navigations never gain network access.
            args.SetResponse(None::<&ICoreWebView2WebResourceResponse>)?;
            return Ok(());
        }
        // SHCreateMemStream copies bytes into an owned stream before borrow release.
        let (resource, remote) = {
            let inner = shared.inner.borrow();
            (inner.document.resource(&uri), inner.document.remote_images)
        };
        let response = resource
            .map(|resource| {
                response(
                    &environment,
                    200,
                    &resource.content_type,
                    &resource.bytes,
                    remote,
                )
            })
            .transpose()?;
        if let Some(response) = response {
            args.SetResponse(&response)?;
        }
        Ok(())
    }
}

unsafe fn response(
    environment: &ICoreWebView2Environment,
    status: i32,
    content_type: &str,
    bytes: &[u8],
    remote: RemoteImagePolicy,
) -> windows::core::Result<ICoreWebView2WebResourceResponse> {
    unsafe {
        let stream =
            SHCreateMemStream(Some(bytes)).ok_or_else(windows::core::Error::from_thread)?;
        let policy = policy(remote);
        let headers = wide(&format!(
            "Content-Type: {content_type}\r\nContent-Security-Policy: {policy}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\n"
        ));
        let reason = wide(if status == 200 { "OK" } else { "Forbidden" });
        environment.CreateWebResourceResponse(
            &stream,
            status,
            PCWSTR(reason.as_ptr()),
            PCWSTR(headers.as_ptr()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::html::InlineImage;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread::JoinHandle,
    };

    impl WebView2Renderer {
        fn set_content(&self, html: &str, inline_images: Vec<InlineImage>) -> Result<(), String> {
            self.render(HtmlDocument {
                html: html.into(),
                inline_images,
                remote_images: RemoteImagePolicy::Block,
            })
        }
    }

    struct ImageServer {
        url: String,
        received: mpsc::Receiver<String>,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }
    impl ImageServer {
        fn new() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}/photo", listener.local_addr().unwrap());
            let (sent, received) = mpsc::channel();
            let stop = Arc::new(AtomicBool::new(false));
            let stopping = stop.clone();
            let thread = std::thread::spawn(move || {
                while !stopping.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(2)))
                                .unwrap();
                            let mut request = Vec::new();
                            let mut bytes = [0; 1024];
                            while request.len() < 8192
                                && !request.windows(4).any(|s| s == b"\r\n\r\n")
                            {
                                match stream.read(&mut bytes) {
                                    Ok(0) | Err(_) => break,
                                    Ok(count) => request.extend_from_slice(&bytes[..count]),
                                }
                            }
                            sent.send(String::from_utf8_lossy(&request).into_owned())
                                .unwrap();
                            let image = include_bytes!(
                                "../../../../../assets/windows/sources/legacy-runtime-atlases/toolbar.png"
                            );
                            let header = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                image.len()
                            );
                            let _ = stream.write_all(header.as_bytes());
                            let _ = stream.write_all(image);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5))
                        }
                        Err(error) => panic!("Local image server failed: {error}"),
                    }
                }
            });
            Self {
                url,
                received,
                stop,
                thread: Some(thread),
            }
        }
    }
    impl Drop for ImageServer {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            self.thread.take().unwrap().join().unwrap();
        }
    }

    struct TestProfile(PathBuf);
    impl TestProfile {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            Self(
                std::env::temp_dir()
                    .join("winmail-html-tests")
                    .join(format!("{}-{stamp}", std::process::id())),
            )
        }
    }
    impl Drop for TestProfile {
        fn drop(&mut self) {
            let Ok(path) = self.0.canonicalize() else {
                return;
            };
            let Ok(root) = std::env::temp_dir()
                .join("winmail-html-tests")
                .canonicalize()
            else {
                return;
            };
            if path.parent() == Some(root.as_path()) {
                // The browser can briefly retain files after Close; cleanup is
                // best effort and only ever targets this test's unique folder.
                let _ = std::fs::remove_dir_all(path);
            }
        }
    }

    struct Parent(HWND);
    impl Drop for Parent {
        fn drop(&mut self) {
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }

    fn pump_until(mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done() {
            assert!(
                Instant::now() < deadline,
                "WebView2 did not finish within 20 seconds"
            );
            unsafe {
                let mut message = std::mem::zeroed();
                while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_ready(view: &WebView2Renderer) {
        pump_until(|| match view.status() {
            HtmlStatus::Ready => true,
            HtmlStatus::Loading => false,
            HtmlStatus::Failed(error) => panic!("HTML renderer failed: {error}"),
        });
    }

    fn inspect(view: &WebView2Renderer, script: &str) -> String {
        let result = Rc::new(RefCell::new(None));
        let captured = result.clone();
        let callback = ExecuteScriptCompletedHandler::create(Box::new(move |status, json| {
            *captured.borrow_mut() = Some(status.map(|()| json).map_err(|error| error.to_string()));
            Ok(())
        }));
        // Host-injected inspection is test-only. Mail scripts remain disabled.
        let webview = view.shared.inner.borrow().webview.clone().unwrap();
        unsafe {
            webview
                .ExecuteScript(PCWSTR(wide(script).as_ptr()), &callback)
                .unwrap();
        }
        pump_until(|| result.borrow().is_some());
        result.borrow_mut().take().unwrap().unwrap()
    }

    #[test]
    fn renders_tables_styles_cid_and_cancels_stale_documents() {
        unsafe {
            // Production sessions and other test executables must never share
            // this browser process/profile, even when tests run concurrently.
            let profile = TestProfile::new();
            let parent = Parent(CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("HTML test").as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                640,
                400,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null_mut()),
                null_mut(),
            ));
            assert!(!parent.0.is_null());
            let view = WebView2Renderer::with_profile(parent.0, parent.0, WM_APP + 114, &profile.0)
                .unwrap();
            MoveWindow(view.hwnd(), 0, 0, 640, 400, 0);
            wait_ready(&view);
            view.set_content("<p>OBSOLETE</p>", vec![]).unwrap();
            view.set_content(r#"<style>.colored{color:rgb(12,34,56)}</style><table id="mail"><tr><td class="colored">CURRENT</td></tr></table><img id="cid" src="cid:test-image"><script>window.mailExecuted=true</script><img src="https://example.invalid/tracker" onerror="window.mailExecuted=true">"#, vec![InlineImage {
                content_id: "test-image".into(), content_type: "image/png".into(),
                bytes: include_bytes!("../../../../../assets/windows/sources/legacy-runtime-atlases/toolbar.png").to_vec(),
            }]).unwrap();
            wait_ready(&view);
            let dom = inspect(
                &view,
                "[document.querySelector('#mail').textContent,document.querySelectorAll('table').length,getComputedStyle(document.querySelector('.colored')).color,document.querySelector('#cid').naturalWidth,typeof window.mailExecuted,document.querySelectorAll('script,iframe,form').length,document.querySelectorAll('img[src^=\"https://example.invalid\"]').length]",
            );
            assert_eq!(
                dom,
                r#"["CURRENT",1,"rgb(12, 34, 56)",1024,"undefined",0,0]"#
            );
            let webview = view.shared.inner.borrow().webview.clone().unwrap();
            let settings = webview.Settings().unwrap();
            let mut enabled = BOOL::default();
            settings.IsScriptEnabled(&mut enabled).unwrap();
            assert!(!enabled.as_bool());
            settings.AreHostObjectsAllowed(&mut enabled).unwrap();
            assert!(!enabled.as_bool());
            settings.IsWebMessageEnabled(&mut enabled).unwrap();
            assert!(!enabled.as_bool());
            settings
                .AreDefaultContextMenusEnabled(&mut enabled)
                .unwrap();
            assert!(enabled.as_bool());
            // Programmatic navigation cannot leave the offline message origin.
            let blocked = view.shared.inner.borrow().blocked_navigations;
            webview
                .Navigate(PCWSTR(wide("https://example.invalid/blocked").as_ptr()))
                .unwrap();
            pump_until(|| view.shared.inner.borrow().blocked_navigations > blocked);
            let mut uri = PWSTR::null();
            webview.Source(&mut uri).unwrap();
            assert!(take_pwstr(uri).starts_with("https://winmail.invalid/message/2/"));
            assert_eq!(
                inspect(&view, "document.querySelector('#mail').textContent"),
                r#""CURRENT""#
            );
            let server = ImageServer::new();
            let remote = format!(
                r#"<img id="remote" src="{}"><style>@import url({}/forbidden-css);@font-face{{font-family:X;src:url({}/forbidden-font)}}</style><script>fetch('{}/forbidden-fetch')</script>"#,
                server.url, server.url, server.url, server.url
            );
            view.render(HtmlDocument {
                html: remote.clone(),
                inline_images: vec![],
                remote_images: RemoteImagePolicy::Block,
            })
            .unwrap();
            wait_ready(&view);
            assert!(view.has_blocked_images());
            assert_eq!(
                inspect(&view, "document.querySelector('#remote').naturalWidth"),
                "0"
            );
            assert!(
                server.received.try_recv().is_err(),
                "Blocked mail requested a remote image"
            );
            view.render(HtmlDocument {
                html: remote,
                inline_images: vec![],
                remote_images: RemoteImagePolicy::Allow,
            })
            .unwrap();
            wait_ready(&view);
            assert!(!view.has_blocked_images());
            assert_eq!(
                inspect(&view, "document.querySelector('#remote').naturalWidth"),
                "1024"
            );
            let request = server
                .received
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            assert!(request.starts_with("GET /photo HTTP/1.1\r\n"), "{request}");
            assert!(
                !request.to_ascii_lowercase().contains("\r\nreferer:"),
                "Image request leaked the mail origin"
            );
            assert!(
                server.received.try_recv().is_err(),
                "Consent permitted a non-image request"
            );
            view.set_content("", vec![]).unwrap();
            wait_ready(&view);
            assert_eq!(inspect(&view, "document.body.textContent"), r#""""#);
            let host = view.hwnd();
            drop(settings);
            drop(webview);
            drop(view);
            assert_eq!(IsWindow(host), 0);
            // Async completion after an immediately dropped host is harmless.
            let cancelled_profile = TestProfile::new();
            let cancelled = WebView2Renderer::with_profile(
                parent.0,
                parent.0,
                WM_APP + 114,
                &cancelled_profile.0,
            )
            .unwrap();
            let weak = Rc::downgrade(&cancelled.shared);
            drop(cancelled);
            assert!(weak.upgrade().is_none());
        }
    }
}
