//! OLE site and per-document security. These settings never change the registry.

use super::ui_handler::{IDocHostUIHandler, IDocHostUIHandler_Impl};
use super::{State, dispatch};
use crate::{html::RemoteImagePolicy, strings::wide};
use std::{
    ffi::c_void,
    ptr::null_mut,
    rc::{Rc, Weak},
};
use windows::{
    Win32::{
        Foundation::*,
        System::{Com::Urlmon::*, Com::*, Ole::*, Variant::VARIANT},
        UI::WindowsAndMessaging::{HMENU, MSG},
    },
    core::{BOOL, GUID, IUnknownImpl, Interface, OutRef, PCWSTR, Ref, implement},
};

pub(super) const DISPID_DOWNLOAD_CONTROL: i32 = -5512;
// mshtmdid.h: allow images, but never scripts, Java, ActiveX, CSS behaviors,
// frames, meta refresh, or audio/video downloads. Block also forces offline.
const DOWNLOAD_CONTROL: u32 =
    0x10 | 0x80 | 0x100 | 0x200 | 0x400 | 0x1000 | 0x8000 | 0x80000 | 0x20000000 | 0x40000000;
const FORCE_OFFLINE: u32 = 0x10000000;

#[implement(
    IOleClientSite,
    IOleInPlaceSite,
    IOleInPlaceFrame,
    IDispatch,
    IServiceProvider,
    IInternetHostSecurityManager,
    IDocHostUIHandler
)]
pub(super) struct Site {
    pub(super) state: Weak<State>,
}

impl IDocHostUIHandler_Impl for Site_Impl {}

impl Site_Impl {
    fn state(&self) -> windows::core::Result<Rc<State>> {
        self.state
            .upgrade()
            .filter(|state| !state.disposed.get())
            .ok_or_else(|| Error::from_hresult(E_FAIL))
    }
}

use windows::core::Error;

impl IOleClientSite_Impl for Site_Impl {
    fn SaveObject(&self) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn GetMoniker(&self, _: &OLEGETMONIKER, _: &OLEWHICHMK) -> windows::core::Result<IMoniker> {
        Err(E_NOTIMPL.into())
    }
    fn GetContainer(&self) -> windows::core::Result<IOleContainer> {
        Err(E_NOINTERFACE.into())
    }
    fn ShowObject(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnShowWindow(&self, _: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
    fn RequestNewObjectLayout(&self) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
}

impl IOleWindow_Impl for Site_Impl {
    fn GetWindow(&self) -> windows::core::Result<HWND> {
        Ok(HWND(self.state()?.hwnd.get()))
    }
    fn ContextSensitiveHelp(&self, _: BOOL) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
}

impl IOleInPlaceSite_Impl for Site_Impl {
    fn CanInPlaceActivate(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnInPlaceActivate(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnUIActivate(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn GetWindowContext(
        &self,
        frame: OutRef<IOleInPlaceFrame>,
        document: OutRef<IOleInPlaceUIWindow>,
        position: *mut RECT,
        clip: *mut RECT,
        info: *mut OLEINPLACEFRAMEINFO,
    ) -> windows::core::Result<()> {
        let state = self.state()?;
        let interface: IOleInPlaceFrame = self.to_interface();
        frame.write(Some(interface))?;
        document.write(None)?;
        if position.is_null() || clip.is_null() || info.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe {
            position.write(state.rect());
            clip.write(state.rect());
            info.write(OLEINPLACEFRAMEINFO {
                cb: std::mem::size_of::<OLEINPLACEFRAMEINFO>() as u32,
                fMDIApp: BOOL(0),
                hwndFrame: HWND(state.hwnd.get()),
                ..Default::default()
            });
        }
        Ok(())
    }
    fn Scroll(&self, _: &SIZE) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnUIDeactivate(&self, _: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnInPlaceDeactivate(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn DiscardUndoState(&self) -> windows::core::Result<()> {
        Ok(())
    }
    fn DeactivateAndUndo(&self) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn OnPosRectChange(&self, rect: *const RECT) -> windows::core::Result<()> {
        if rect.is_null() {
            return Err(E_POINTER.into());
        }
        // Layout is controlled by the host window's WM_SIZE. MSHTML may call
        // this while SetRect is already running; do not recurse into it.
        Ok(())
    }
}

impl IOleInPlaceUIWindow_Impl for Site_Impl {
    fn GetBorder(&self) -> windows::core::Result<RECT> {
        Ok(self.state()?.rect())
    }
    fn RequestBorderSpace(&self, _: *const RECT) -> windows::core::Result<()> {
        Err(INPLACE_E_NOTOOLSPACE.into())
    }
    fn SetBorderSpace(&self, _: *const RECT) -> windows::core::Result<()> {
        Ok(())
    }
    fn SetActiveObject(
        &self,
        _: Ref<IOleInPlaceActiveObject>,
        _: &PCWSTR,
    ) -> windows::core::Result<()> {
        Ok(())
    }
}

impl IOleInPlaceFrame_Impl for Site_Impl {
    fn InsertMenus(&self, _: HMENU, _: *mut OLEMENUGROUPWIDTHS) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn SetMenu(&self, _: HMENU, _: isize, _: HWND) -> windows::core::Result<()> {
        Ok(())
    }
    fn RemoveMenus(&self, _: HMENU) -> windows::core::Result<()> {
        Ok(())
    }
    fn SetStatusText(&self, _: &PCWSTR) -> windows::core::Result<()> {
        Ok(())
    }
    fn EnableModeless(&self, _: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
    fn TranslateAccelerator(&self, _: *const MSG, _: u16) -> windows::core::Result<()> {
        Err(S_FALSE.into())
    }
}

impl IDispatch_Impl for Site_Impl {
    fn GetTypeInfoCount(&self) -> windows::core::Result<u32> {
        Ok(0)
    }
    fn GetTypeInfo(&self, _: u32, _: u32) -> windows::core::Result<ITypeInfo> {
        Err(E_NOTIMPL.into())
    }
    fn GetIDsOfNames(
        &self,
        _: *const GUID,
        _: *const PCWSTR,
        _: u32,
        _: u32,
        _: *mut i32,
    ) -> windows::core::Result<()> {
        Err(DISP_E_UNKNOWNNAME.into())
    }
    fn Invoke(
        &self,
        member: i32,
        _: *const GUID,
        _: u32,
        _: DISPATCH_FLAGS,
        _: *const DISPPARAMS,
        result: *mut VARIANT,
        _: *mut EXCEPINFO,
        _: *mut u32,
    ) -> windows::core::Result<()> {
        if member != DISPID_DOWNLOAD_CONTROL {
            return Err(DISP_E_MEMBERNOTFOUND.into());
        }
        if result.is_null() {
            return Err(E_POINTER.into());
        }
        let offline = self.state()?.remote_images.get() == RemoteImagePolicy::Block;
        unsafe {
            result.write(VARIANT::from(
                (DOWNLOAD_CONTROL | if offline { FORCE_OFFLINE } else { 0 }) as i32,
            ));
        }
        Ok(())
    }
}

impl IServiceProvider_Impl for Site_Impl {
    fn QueryService(
        &self,
        service: *const GUID,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> windows::core::Result<()> {
        if service.is_null() || iid.is_null() || object.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe {
            object.write(null_mut());
            if *service == IInternetHostSecurityManager::IID
                && *iid == IInternetHostSecurityManager::IID
            {
                let interface: IInternetHostSecurityManager = self.to_interface();
                object.write(interface.into_raw());
                Ok(())
            } else {
                Err(E_NOINTERFACE.into())
            }
        }
    }
}

impl IInternetHostSecurityManager_Impl for Site_Impl {
    fn GetSecurityId(&self, _: *mut u8, _: *mut u32, _: usize) -> windows::core::Result<()> {
        Err(windows::core::HRESULT(INET_E_DEFAULT_ACTION).into())
    }
    fn ProcessUrlAction(
        &self,
        _: u32,
        policy: *mut u8,
        length: u32,
        _: *const u8,
        _: u32,
        _: u32,
        _: u32,
    ) -> windows::core::Result<()> {
        if policy.is_null() || length < 4 {
            return Err(E_INVALIDARG.into());
        }
        unsafe {
            policy.cast::<u32>().write_unaligned(URLPOLICY_DISALLOW);
        }
        Ok(())
    }
    fn QueryCustomPolicy(
        &self,
        _: *const GUID,
        policy: *mut *mut u8,
        length: *mut u32,
        _: *const u8,
        _: u32,
        _: u32,
    ) -> windows::core::Result<()> {
        if !policy.is_null() {
            unsafe {
                policy.write(null_mut());
            }
        }
        if !length.is_null() {
            unsafe {
                length.write(0);
            }
        }
        Err(E_ACCESSDENIED.into())
    }
}

#[implement(IDispatch)]
pub(super) struct ClickHandler {
    pub(super) state: Weak<State>,
}

impl IDispatch_Impl for ClickHandler_Impl {
    fn GetTypeInfoCount(&self) -> windows::core::Result<u32> {
        Ok(0)
    }
    fn GetTypeInfo(&self, _: u32, _: u32) -> windows::core::Result<ITypeInfo> {
        Err(E_NOTIMPL.into())
    }
    fn GetIDsOfNames(
        &self,
        _: *const GUID,
        _: *const PCWSTR,
        _: u32,
        _: u32,
        _: *mut i32,
    ) -> windows::core::Result<()> {
        Err(DISP_E_UNKNOWNNAME.into())
    }
    fn Invoke(
        &self,
        _: i32,
        _: *const GUID,
        _: u32,
        _: DISPATCH_FLAGS,
        _: *const DISPPARAMS,
        result: *mut VARIANT,
        _: *mut EXCEPINFO,
        _: *mut u32,
    ) -> windows::core::Result<()> {
        if !result.is_null() {
            unsafe {
                result.write(VARIANT::from(false));
            }
        }
        let Some(state) = self.state.upgrade().filter(|state| !state.disposed.get()) else {
            return Ok(());
        };
        let Some(document) = state.document.borrow().clone() else {
            return Ok(());
        };
        let window = dispatch::object(&document, "parentWindow")?;
        let event = dispatch::object(&window, "event")?;
        dispatch::set(&event, "returnValue", false.into())?;
        let mut element = dispatch::object(&event, "srcElement")?;
        for _ in 0..64 {
            if dispatch::text(&element, "tagName").is_ok_and(|tag| tag.eq_ignore_ascii_case("a")) {
                let href = dispatch::text(&element, "href")?;
                if let Some(href) = super::super::content::external_url(&href) {
                    let verb = wide("open");
                    let target = wide(&href);
                    unsafe {
                        windows_sys::Win32::UI::Shell::ShellExecuteW(
                            state.hwnd.get(),
                            verb.as_ptr(),
                            target.as_ptr(),
                            null_mut(),
                            null_mut(),
                            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
                        );
                    }
                }
                break;
            }
            match dispatch::object(&element, "parentElement") {
                Ok(parent) => element = parent,
                Err(_) => break,
            }
        }
        Ok(())
    }
}
