//! A direct OLE document host for the Windows MSHTML component.
//!
//! No browser executable, ATL dependency, or global Internet Explorer settings
//! are needed. Each host owns its document and supplies its own security policy.

mod dispatch;
mod site;
#[cfg(test)]
mod tests;
mod ui_handler;

use super::{
    HostedHtmlRenderer, HtmlDocument, HtmlRenderer, HtmlStatus, RemoteImagePolicy,
    content::prepare_mshtml,
};
use crate::strings::wide;
use std::{
    cell::{Cell, RefCell},
    ptr::null_mut,
    rc::Rc,
    sync::OnceLock,
};
use windows::{
    Win32::{
        Foundation::RECT as ComRect,
        System::{
            Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, IDispatch, IPersistStreamInit},
            Ole::*,
        },
        UI::Shell::SHCreateMemStream,
    },
    core::{GUID, Interface},
};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::WindowsAndMessaging::*,
};

const CLASS: &str = "WinMailRustMshtmlPreview";
const HTML_DOCUMENT: GUID = GUID::from_u128(0x25336920_03f9_11cf_8fd0_00aa00686f13);
const POLL: usize = 1;

pub(super) struct State {
    hwnd: Cell<HWND>,
    notify: HWND,
    message: u32,
    status: RefCell<HtmlStatus>,
    blocked_images: Cell<bool>,
    remote_images: Cell<RemoteImagePolicy>,
    document: RefCell<Option<IDispatch>>,
    view: RefCell<Option<IOleDocumentView>>,
    ole: RefCell<Option<IOleObject>>,
    site: RefCell<Option<IOleClientSite>>,
    disposed: Cell<bool>,
    window_attached: Cell<bool>,
}

impl State {
    fn rect(&self) -> ComRect {
        let mut rect = windows_sys::Win32::Foundation::RECT::default();
        unsafe {
            GetClientRect(self.hwnd.get(), &mut rect);
        }
        ComRect {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }

    fn resize(&self) {
        let view = self.view.borrow().clone();
        if let Some(view) = view {
            unsafe {
                let _ = view.SetRect(&self.rect());
            }
        }
    }

    fn focus(&self) {
        let ole = self.ole.borrow().clone();
        if let Some(ole) = ole
            && let Ok(window) = ole.cast::<IOleInPlaceObject>()
            && let Ok(hwnd) = unsafe { window.GetWindow() }
            && hwnd.0 != self.hwnd.get()
        {
            unsafe {
                windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(hwnd.0);
            }
        }
    }

    fn set_status(&self, status: HtmlStatus) {
        *self.status.borrow_mut() = status;
        unsafe {
            PostMessageW(self.notify, self.message, 0, 0);
        }
    }

    fn poll(self: &Rc<Self>) {
        if self.disposed.get() {
            return;
        }
        let document = self.document.borrow().clone();
        let Some(document) = document else {
            return;
        };
        if dispatch::text(&document, "readyState").is_ok_and(|state| state == "complete") {
            // The Rust event handler cancels default link navigation. Mail cannot
            // inject this callback: script and all event attributes were removed.
            let handler: IDispatch = site::ClickHandler {
                state: Rc::downgrade(self),
            }
            .into();
            let _ = dispatch::set(&document, "onclick", handler.into());
            unsafe {
                KillTimer(self.hwnd.get(), POLL);
            }
            self.set_status(HtmlStatus::Ready);
        }
    }
}

/// Owned OLE document and child window, confined to its creating UI thread.
pub(super) struct MshtmlRenderer {
    state: Rc<State>,
    _apartment: Apartment,
}

impl MshtmlRenderer {
    pub(super) unsafe fn new(parent: HWND, notify: HWND, message: u32) -> Result<Self, String> {
        let thread = unsafe { GetCurrentThreadId() };
        if parent.is_null()
            || notify.is_null()
            || message < WM_APP
            || unsafe { GetWindowThreadProcessId(parent, null_mut()) } != thread
            || unsafe { GetWindowThreadProcessId(notify, null_mut()) } != thread
        {
            return Err(
                "MSHTML requires live windows on the UI thread and a WM_APP message".into(),
            );
        }
        let apartment = Apartment::new()?;
        register_class()?;
        let state = Rc::new(State {
            hwnd: Cell::new(null_mut()),
            notify,
            message,
            status: RefCell::new(HtmlStatus::Loading),
            blocked_images: Cell::new(false),
            remote_images: Cell::new(RemoteImagePolicy::Block),
            document: RefCell::new(None),
            view: RefCell::new(None),
            ole: RefCell::new(None),
            site: RefCell::new(None),
            disposed: Cell::new(false),
            window_attached: Cell::new(false),
        });
        let raw = Rc::into_raw(state.clone());
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_CONTROLPARENT,
                wide(CLASS).as_ptr(),
                wide("").as_ptr(),
                WS_CHILD | WS_TABSTOP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                0,
                0,
                1,
                1,
                parent,
                null_mut(),
                GetModuleHandleW(null_mut()),
                raw.cast(),
            )
        };
        if hwnd.is_null() {
            // Once WM_NCCREATE runs, WM_NCDESTROY releases the window's Rc even
            // if creation subsequently fails.
            if !state.window_attached.get() {
                unsafe {
                    drop(Rc::from_raw(raw));
                }
            }
            return Err(format!(
                "MSHTML host window: {}",
                std::io::Error::last_os_error()
            ));
        }
        state.hwnd.set(hwnd);
        let renderer = Self {
            state,
            _apartment: apartment,
        };
        renderer
            .initialize()
            .map_err(|error| format!("MSHTML initialization: {error}"))?;
        Ok(renderer)
    }

    fn initialize(&self) -> windows::core::Result<()> {
        unsafe {
            let ole: IOleObject = CoCreateInstance(&HTML_DOCUMENT, None, CLSCTX_INPROC_SERVER)?;
            let site: IOleClientSite = site::Site {
                state: Rc::downgrade(&self.state),
            }
            .into();
            ole.SetClientSite(&site)?;
            *self.state.ole.borrow_mut() = Some(ole.clone());
            *self.state.site.borrow_mut() = Some(site.clone());
            let persist: IPersistStreamInit = ole.cast()?;
            persist.InitNew()?;
            let document: IDispatch = ole.cast()?;
            *self.state.document.borrow_mut() = Some(document);
            let doc: IOleDocument = ole.cast()?;
            let inplace: IOleInPlaceSite = site.cast()?;
            let view = doc.CreateView(&inplace, None, 0)?;
            *self.state.view.borrow_mut() = Some(view.clone());
            view.SetRect(&self.state.rect())?;
            view.UIActivate(true)?;
            view.Show(true)?;
            OleSetContainedObject(&ole, true)?;
        }
        Ok(())
    }
}

impl HtmlRenderer for MshtmlRenderer {
    fn render(&self, document: HtmlDocument) -> Result<(), String> {
        let prepared = prepare_mshtml(&document).inspect_err(|error| {
            self.state.blocked_images.set(false);
            self.state.set_status(HtmlStatus::Failed(error.clone()));
        })?;
        self.state.remote_images.set(document.remote_images);
        self.state.blocked_images.set(prepared.blocked_images);
        self.state.set_status(HtmlStatus::Loading);
        // UTF-8 BOM makes IStream loading independent of the system ANSI code page.
        let mut bytes = vec![0xef, 0xbb, 0xbf];
        bytes.extend_from_slice(prepared.html.as_bytes());
        let result = (|| -> windows::core::Result<()> {
            let ole = self.state.ole.borrow().clone().ok_or_else(|| {
                windows::core::Error::from_hresult(windows::Win32::Foundation::E_FAIL)
            })?;
            unsafe {
                if let Ok(control) = ole.cast::<IOleControl>() {
                    let _ = control.OnAmbientPropertyChange(site::DISPID_DOWNLOAD_CONTROL);
                }
                let stream = SHCreateMemStream(Some(&bytes)).ok_or_else(|| {
                    windows::core::Error::from_hresult(windows::Win32::Foundation::E_OUTOFMEMORY)
                })?;
                let persist: IPersistStreamInit = ole.cast()?;
                persist.Load(&stream)?;
                // Query the document again after Load; the previous DOM is obsolete.
                *self.state.document.borrow_mut() = Some(ole.cast()?);
                SetTimer(self.state.hwnd.get(), POLL, 20, None);
            }
            self.state.poll();
            Ok(())
        })();
        result.map_err(|error| {
            let message = format!("MSHTML document: {error}");
            self.state.set_status(HtmlStatus::Failed(message.clone()));
            message
        })
    }

    fn status(&self) -> HtmlStatus {
        self.state.status.borrow().clone()
    }
    fn has_blocked_images(&self) -> bool {
        self.state.blocked_images.get()
    }
}

impl HostedHtmlRenderer for MshtmlRenderer {
    fn hwnd(&self) -> HWND {
        self.state.hwnd.get()
    }
}

impl Drop for MshtmlRenderer {
    fn drop(&mut self) {
        self.state.disposed.set(true);
        unsafe {
            KillTimer(self.state.hwnd.get(), POLL);
            let view = self.state.view.borrow_mut().take();
            if let Some(view) = view {
                let _ = view.Show(false);
                let _ = view.UIActivate(false);
                let _ = view.CloseView(0);
                let _ = view.SetInPlaceSite(None);
            }
            self.state.document.borrow_mut().take();
            let ole = self.state.ole.borrow_mut().take();
            if let Some(ole) = ole {
                let _ = ole.Close(OLECLOSE_NOSAVE);
                let _ = ole.SetClientSite(None);
            }
            self.state.site.borrow_mut().take();
            let hwnd = self.state.hwnd.replace(null_mut());
            if !hwnd.is_null() {
                DestroyWindow(hwnd);
            }
        }
    }
}

struct Apartment;
impl Apartment {
    fn new() -> Result<Self, String> {
        unsafe { OleInitialize(None) }.map_err(|error| format!("MSHTML UI apartment: {error}"))?;
        Ok(Self)
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            OleUninitialize();
        }
    }
}

fn register_class() -> Result<(), String> {
    static REGISTERED: OnceLock<Result<(), String>> = OnceLock::new();
    REGISTERED
        .get_or_init(|| unsafe {
            let name = wide(CLASS);
            let class = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: GetModuleHandleW(null_mut()),
                lpszClassName: name.as_ptr(),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 {
                Err(format!(
                    "MSHTML window class: {}",
                    std::io::Error::last_os_error()
                ))
            } else {
                Ok(())
            }
        })
        .clone()
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lparam as *const CREATESTRUCTW);
            (*(create.lpCreateParams as *const State))
                .window_attached
                .set(true);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as _);
        }
        let raw = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const State;
        if !raw.is_null() {
            Rc::increment_strong_count(raw);
            let state = Rc::from_raw(raw);
            match message {
                WM_SIZE => state.resize(),
                WM_SETFOCUS => state.focus(),
                WM_TIMER if wparam == POLL => state.poll(),
                WM_NCDESTROY => {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(Rc::from_raw(raw));
                }
                _ => {}
            }
        }
        DefWindowProcW(hwnd, message, wparam, lparam)
    }
}
