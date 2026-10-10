//! User-scoped Windows DPAPI protection. The serialized encoding belongs to the
//! account model; this module only protects and restores the secret bytes.

use windows_sys::Win32::{
    Foundation::{GetLastError, LocalFree},
    Security::Cryptography::*,
};

pub fn protect_secret(bytes: &[u8]) -> Result<Vec<u8>, String> {
    crypt(bytes, true)
}

pub fn unprotect_secret(bytes: &[u8]) -> Result<Vec<u8>, String> {
    crypt(bytes, false)
}

fn crypt(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len().try_into().map_err(|_| "Secret too large")?,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: The input is a live borrowed byte slice for the synchronous DPAPI
    // call. The output descriptor is initialized and supplied for DPAPI to fill.
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        // SAFETY: GetLastError has no pointer or lifetime preconditions.
        return Err(format!("Windows DPAPI failed: {}", unsafe {
            GetLastError()
        }));
    }
    // SAFETY: On success DPAPI returns cbData initialized bytes in pbData. Copy
    // them before releasing the allocation with its prescribed LocalFree API.
    let data = if output.cbData == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() }
    };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(data)
}
