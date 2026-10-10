//! The retired IDocHostUIHandler ABI, defined from the Windows SDK MsHtmHst.h.
//! It suppresses browser chrome and supplies no script-visible host automation.
#![allow(non_snake_case, non_camel_case_types)]

use std::{ffi::c_void, ptr::null_mut};
use windows::{
    Win32::{
        Foundation::{E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, POINT, RECT, S_FALSE, S_OK},
        System::Com::CoTaskMemAlloc,
        UI::WindowsAndMessaging::MSG,
    },
    core::{BOOL, GUID, HRESULT, IUnknown_Vtbl, IUnknownImpl, Interface, PWSTR},
};

windows_core::imp::define_interface!(
    IDocHostUIHandler,
    IDocHostUIHandler_Vtbl,
    0xbd3f23c0_d43e_11cf_893b_00aa00bdce1a
);
windows_core::imp::interface_hierarchy!(IDocHostUIHandler, windows_core::IUnknown);

#[repr(C)]
struct HostInfo {
    size: u32,
    flags: u32,
    double_click: u32,
    css: PWSTR,
    namespace: PWSTR,
}

#[repr(C)]
pub struct IDocHostUIHandler_Vtbl {
    base__: IUnknown_Vtbl,
    ShowContextMenu: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *const POINT,
        *mut c_void,
        *mut c_void,
    ) -> HRESULT,
    GetHostInfo: unsafe extern "system" fn(*mut c_void, *mut HostInfo) -> HRESULT,
    ShowUI: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut c_void,
    ) -> HRESULT,
    HideUI: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    UpdateUI: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    EnableModeless: unsafe extern "system" fn(*mut c_void, BOOL) -> HRESULT,
    OnDocWindowActivate: unsafe extern "system" fn(*mut c_void, BOOL) -> HRESULT,
    OnFrameWindowActivate: unsafe extern "system" fn(*mut c_void, BOOL) -> HRESULT,
    ResizeBorder: unsafe extern "system" fn(*mut c_void, *const RECT, *mut c_void, BOOL) -> HRESULT,
    TranslateAccelerator:
        unsafe extern "system" fn(*mut c_void, *const MSG, *const GUID, u32) -> HRESULT,
    GetOptionKeyPath: unsafe extern "system" fn(*mut c_void, *mut PWSTR, u32) -> HRESULT,
    GetDropTarget: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
    GetExternal: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HRESULT,
    TranslateUrl: unsafe extern "system" fn(*mut c_void, u32, PWSTR, *mut PWSTR) -> HRESULT,
    FilterDataObject:
        unsafe extern "system" fn(*mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT,
}

pub trait IDocHostUIHandler_Impl: IUnknownImpl {}

impl IDocHostUIHandler_Vtbl {
    pub const fn new<Identity: IDocHostUIHandler_Impl, const OFFSET: isize>() -> Self {
        Self {
            base__: IUnknown_Vtbl::new::<Identity, OFFSET>(),
            ShowContextMenu: context_menu,
            GetHostInfo: host_info,
            ShowUI: show_ui,
            HideUI: okay,
            UpdateUI: okay,
            EnableModeless: okay_bool,
            OnDocWindowActivate: okay_bool,
            OnFrameWindowActivate: okay_bool,
            ResizeBorder: resize_border,
            TranslateAccelerator: accelerator,
            GetOptionKeyPath: option_key,
            GetDropTarget: no_data,
            GetExternal: no_external,
            TranslateUrl: translate_url,
            FilterDataObject: no_data,
        }
    }
    pub fn matches(iid: &GUID) -> bool {
        iid == &IDocHostUIHandler::IID
    }
}

unsafe extern "system" fn context_menu(
    _: *mut c_void,
    _: u32,
    _: *const POINT,
    _: *mut c_void,
    _: *mut c_void,
) -> HRESULT {
    S_OK
}
unsafe extern "system" fn host_info(_: *mut c_void, info: *mut HostInfo) -> HRESULT {
    if info.is_null() {
        return E_POINTER;
    }
    unsafe {
        (*info).flags = 0x2 | 0x4 | 0x10 | 0x40000000; // help, border, inactive scripts, DPI-aware
        (*info).double_click = 0;
        (*info).css = PWSTR::null();
        (*info).namespace = PWSTR::null();
    }
    S_OK
}
unsafe extern "system" fn show_ui(
    _: *mut c_void,
    _: u32,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
    _: *mut c_void,
) -> HRESULT {
    S_OK
}
unsafe extern "system" fn okay(_: *mut c_void) -> HRESULT {
    S_OK
}
unsafe extern "system" fn okay_bool(_: *mut c_void, _: BOOL) -> HRESULT {
    S_OK
}
unsafe extern "system" fn resize_border(
    _: *mut c_void,
    _: *const RECT,
    _: *mut c_void,
    _: BOOL,
) -> HRESULT {
    S_OK
}
unsafe extern "system" fn accelerator(
    _: *mut c_void,
    _: *const MSG,
    _: *const GUID,
    _: u32,
) -> HRESULT {
    S_FALSE
}
unsafe extern "system" fn option_key(_: *mut c_void, out: *mut PWSTR, _: u32) -> HRESULT {
    if !out.is_null() {
        unsafe {
            out.write(PWSTR::null());
        }
    }
    E_NOTIMPL
}
unsafe extern "system" fn no_data(
    _: *mut c_void,
    _: *mut c_void,
    out: *mut *mut c_void,
) -> HRESULT {
    if !out.is_null() {
        unsafe {
            out.write(null_mut());
        }
    }
    E_NOTIMPL
}
unsafe extern "system" fn no_external(_: *mut c_void, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    unsafe {
        out.write(null_mut());
    }
    S_OK
}
unsafe extern "system" fn translate_url(
    _: *mut c_void,
    _: u32,
    _: PWSTR,
    out: *mut PWSTR,
) -> HRESULT {
    if out.is_null() {
        return E_POINTER;
    }
    unsafe {
        out.write(PWSTR::null());
    }
    // User links are handled by the Rust onclick callback. Every attempted
    // in-place navigation is replaced by an inert URL independently of it.
    let blank: Vec<u16> = "about:blank".encode_utf16().chain(Some(0)).collect();
    let memory = unsafe { CoTaskMemAlloc(blank.len() * 2) }.cast::<u16>();
    if memory.is_null() {
        return E_OUTOFMEMORY;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(blank.as_ptr(), memory, blank.len());
        out.write(PWSTR(memory));
    }
    S_OK
}
