//! Native image lists for the embedded toolbar and folder artwork.
//!
//! All pixels passed to Windows are premultiplied BGRA. Both sets share the
//! independent-tile PNG decoder and resizer.

mod atlas;

use std::ptr::null_mut;
use windows_sys::Win32::{Graphics::Gdi::*, UI::Controls::*};

/// Artwork set with its original row-major icon indices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSet {
    Toolbar,
    Folders,
    FolderTree,
}

impl IconSet {
    pub const fn count(self) -> usize {
        let (columns, rows) = self.grid();
        columns * rows
    }

    const fn grid(self) -> (usize, usize) {
        match self {
            Self::Toolbar => (8, 8),
            Self::Folders => (10, 6),
            Self::FolderTree => (3, 3),
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
/// Both artwork sets use their embedded PNG and preserve the original indices.
/// Returns null if decoding or native image-list creation fails.
///
/// # Safety
/// Call on the UI thread. Keep the returned list alive while attached to any
/// control and release it with `ImageList_Destroy` after use.
pub unsafe fn images(set: IconSet, size: i32) -> HIMAGELIST {
    let size = size.clamp(16, 256) as usize;
    let count = set.count();
    let Ok(strip) = atlas::strip(set, size) else {
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
