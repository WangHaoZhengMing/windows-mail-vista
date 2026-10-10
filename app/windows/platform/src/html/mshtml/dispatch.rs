//! Small late-bound DOM adapter. MSHTML's retired interfaces are not in win32metadata.

use windows::{
    Win32::System::{Com::*, Ole::DISPID_PROPERTYPUT, Variant::VARIANT},
    core::{BSTR, GUID, PCWSTR},
};

pub(super) fn invoke(
    object: &IDispatch,
    name: &str,
    flags: DISPATCH_FLAGS,
    args: &mut [VARIANT],
) -> windows::core::Result<VARIANT> {
    let name: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
    let mut id = 0;
    let mut named = DISPID_PROPERTYPUT;
    unsafe {
        object.GetIDsOfNames(&GUID::zeroed(), &PCWSTR(name.as_ptr()), 1, 0, &mut id)?;
        let params = DISPPARAMS {
            rgvarg: args.as_mut_ptr(),
            cArgs: args.len() as u32,
            rgdispidNamedArgs: if flags == DISPATCH_PROPERTYPUT {
                &mut named
            } else {
                std::ptr::null_mut()
            },
            cNamedArgs: u32::from(flags == DISPATCH_PROPERTYPUT),
        };
        let mut value = VARIANT::default();
        object.Invoke(
            id,
            &GUID::zeroed(),
            0,
            flags,
            &params,
            Some(&mut value),
            None,
            None,
        )?;
        Ok(value)
    }
}

pub(super) fn get(object: &IDispatch, name: &str) -> windows::core::Result<VARIANT> {
    invoke(object, name, DISPATCH_PROPERTYGET, &mut [])
}

pub(super) fn set(object: &IDispatch, name: &str, value: VARIANT) -> windows::core::Result<()> {
    invoke(object, name, DISPATCH_PROPERTYPUT, &mut [value]).map(|_| ())
}

pub(super) fn object(object: &IDispatch, name: &str) -> windows::core::Result<IDispatch> {
    IDispatch::try_from(&get(object, name)?)
}

pub(super) fn text(object: &IDispatch, name: &str) -> windows::core::Result<String> {
    Ok(BSTR::try_from(&get(object, name)?)?.to_string())
}
