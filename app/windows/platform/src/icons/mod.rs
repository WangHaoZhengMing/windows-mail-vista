//! Native image lists built from individually named embedded PNG assets.
//!
//! All pixels passed to Windows are premultiplied BGRA. Both sets share the
//! independent-image PNG decoder and resizer.

mod bitmap;
mod catalog;
pub use catalog::{FolderIcon, ToolbarIcon, TreeIcon};

use std::ptr::null_mut;
use windows_sys::Win32::{
    Graphics::Gdi::*,
    UI::{Controls::*, WindowsAndMessaging::*},
};

/// Named asset collections; index order is managed by the catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSet {
    Toolbar,
    Folders,
    FolderTree,
}

impl IconSet {
    pub const fn count(self) -> usize {
        self.assets().len()
    }
    const fn assets(self) -> &'static [(&'static str, &'static [u8])] {
        match self {
            Self::Toolbar => catalog::TOOLBAR,
            Self::Folders => catalog::MESSAGES,
            Self::FolderTree => catalog::FOLDERS,
        }
    }
}

unsafe fn dib(width: i32, height: i32, data: &[u8]) -> HBITMAP {
    unsafe {
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: 40,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(null_mut(), &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if !bitmap.is_null() {
            std::ptr::copy_nonoverlapping(data.as_ptr(), bits.cast(), data.len());
        }
        bitmap
    }
}

/// Create an owned native image list at the requested device-pixel size.
/// Each set embeds independent PNGs in catalog order.
/// Returns null if decoding or native image-list creation fails.
///
/// # Safety
/// Call on the UI thread. Keep the returned list alive while attached to any
/// control and release it with `ImageList_Destroy` after use.
pub unsafe fn images(set: IconSet, size: i32) -> HIMAGELIST {
    let size = size.clamp(16, 256) as usize;
    let count = set.count();
    let Ok(strip) = bitmap::strip(set, size) else {
        return 0;
    };
    unsafe {
        let bitmap = dib((size * count) as i32, size as i32, &strip);
        if bitmap.is_null() {
            return 0;
        }
        let list = ImageList_Create(size as i32, size as i32, ILC_COLOR32, count as i32, 1);
        if list == 0 {
            DeleteObject(bitmap);
            return 0;
        }
        let added = ImageList_Add(list, bitmap, null_mut());
        DeleteObject(bitmap);
        if added != 0 || ImageList_GetImageCount(list) != count as i32 {
            ImageList_Destroy(list);
            return 0;
        }
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_lists_keep_each_sets_count_and_size_at_each_scale() {
        unsafe {
            for size in [16, 20, 24, 28, 32, 48, 64, 128, 256] {
                for set in [IconSet::Toolbar, IconSet::Folders, IconSet::FolderTree] {
                    let list = images(set, size);
                    assert_ne!(list, 0, "set={set:?}, size={size}");
                    assert_eq!(ImageList_GetImageCount(list), set.count() as i32);
                    let mut width = 0;
                    let mut height = 0;
                    assert_ne!(ImageList_GetIconSize(list, &mut width, &mut height), 0);
                    assert_eq!((width, height), (size, size));
                    assert_ne!(ImageList_Destroy(list), 0);
                }
            }
        }
    }
}

/// Create an owned application icon; release the result with `DestroyIcon`.
///
/// # Safety
/// Call on the UI thread with a positive requested pixel size.
pub unsafe fn icon(size: i32) -> HICON {
    let bytes = include_bytes!("../../../../../assets/windows/icons/app.ico");
    let count = u16::from_le_bytes(bytes[4..6].try_into().unwrap()) as usize;
    let record = (0..count)
        .map(|i| 6 + i * 16)
        .min_by_key(|&i| {
            let width = if bytes[i] == 0 { 256 } else { bytes[i] as i32 };
            let depth = u16::from_le_bytes(bytes[i + 6..i + 8].try_into().unwrap());
            (width < size, (width - size).abs(), std::cmp::Reverse(depth))
        })
        .unwrap();
    let length = u32::from_le_bytes(bytes[record + 8..record + 12].try_into().unwrap()) as usize;
    let offset = u32::from_le_bytes(bytes[record + 12..record + 16].try_into().unwrap()) as usize;
    unsafe {
        CreateIconFromResourceEx(
            bytes[offset..offset + length].as_ptr(),
            length as u32,
            1,
            0x30000,
            size,
            size,
            LR_DEFAULTCOLOR,
        )
    }
}

/// Create an owned image list; release it with `ImageList_Destroy` after use.
///
/// # Safety
/// Call on the UI thread with the target window's DPI.
pub unsafe fn images_for_dpi(set: IconSet, dpi: u32) -> HIMAGELIST {
    let logical_size = if set == IconSet::Toolbar { 20 } else { 16 };
    unsafe { images(set, super::dpi::scale(logical_size, dpi)) }
}
