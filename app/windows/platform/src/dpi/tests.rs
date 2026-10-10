use super::*;
use std::mem::size_of;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

struct Window(HWND);
impl Window {
    fn new(parent: HWND) -> Self {
        let hwnd = unsafe {
            CreateWindowExW(
                0,
                wide("STATIC").as_ptr(),
                wide("DPI font regression").as_ptr(),
                if parent.is_null() {
                    WS_OVERLAPPEDWINDOW
                } else {
                    WS_CHILD
                },
                0,
                0,
                300,
                100,
                parent,
                null_mut(),
                GetModuleHandleW(null_mut()),
                null_mut(),
            )
        };
        assert!(!hwnd.is_null());
        Self(hwnd)
    }
}
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}
fn control_font(control: HWND) -> HFONT {
    unsafe { SendMessageW(control, WM_GETFONT, 0, 0) as HFONT }
}
fn font_description(font: HFONT) -> LOGFONTW {
    let mut description = LOGFONTW::default();
    assert_eq!(
        unsafe {
            GetObjectW(
                font,
                size_of::<LOGFONTW>() as i32,
                (&mut description as *mut LOGFONTW).cast(),
            )
        },
        size_of::<LOGFONTW>() as i32
    );
    description
}
fn face(description: &LOGFONTW) -> String {
    let end = description
        .lfFaceName
        .iter()
        .position(|character| *character == 0)
        .unwrap();
    String::from_utf16(&description.lfFaceName[..end]).unwrap()
}
fn is_font_alive(font: HFONT) -> bool {
    let mut description = LOGFONTW::default();
    unsafe {
        GetObjectW(
            font,
            size_of::<LOGFONTW>() as i32,
            (&mut description as *mut LOGFONTW).cast(),
        ) != 0
    }
}

#[test]
fn bold_is_opt_in_cached_and_rebuilt_at_the_same_dpi_size_as_regular() {
    let parent = Window::new(null_mut());
    let label = Window::new(parent.0);
    let value = Window::new(parent.0);
    let mut resources = Resources::default();
    unsafe {
        resources.update(parent.0, 96);
    }
    let regular = control_font(value.0);
    assert_eq!(control_font(label.0), regular);
    assert!(resources.bold_font.is_null());
    let normal = font_description(regular);
    assert_eq!(normal.lfWeight, FW_NORMAL as i32);
    assert_eq!(normal.lfHeight, -12);
    assert_eq!(face(&normal), "Segoe UI");

    unsafe {
        resources.apply_font(label.0, FontWeight::Bold);
    }
    let bold = control_font(label.0);
    let bold_description = font_description(bold);
    assert_ne!(bold, regular);
    assert_eq!(bold_description.lfWeight, FW_BOLD as i32);
    assert_eq!(bold_description.lfHeight, normal.lfHeight);
    assert_eq!(face(&bold_description), face(&normal));
    assert_eq!(control_font(value.0), regular);
    for _ in 0..5 {
        unsafe {
            resources.apply_font(label.0, FontWeight::Bold);
        }
        assert_eq!(control_font(label.0), bold);
    }

    for dpi in [192, 144, 96] {
        let old_regular = control_font(value.0);
        let old_bold = control_font(label.0);
        unsafe {
            resources.update(parent.0, dpi);
        }
        assert!(resources.bold_font.is_null());
        assert!(!is_font_alive(old_regular));
        assert!(!is_font_alive(old_bold));
        let regular = control_font(value.0);
        assert_eq!(
            control_font(label.0),
            regular,
            "DPI update restores every child to regular before retiring fonts"
        );
        assert_eq!(font_description(regular).lfHeight, -scale(12, dpi));
        assert_eq!(font_description(regular).lfWeight, FW_NORMAL as i32);
        unsafe {
            resources.apply_font(label.0, FontWeight::Bold);
        }
        let bold = control_font(label.0);
        assert_eq!(font_description(bold).lfHeight, -scale(12, dpi));
        assert_eq!(font_description(bold).lfWeight, FW_BOLD as i32);
    }
    unsafe {
        resources.apply_font(label.0, FontWeight::Regular);
    }
    assert_eq!(control_font(label.0), control_font(value.0));
    let regular = resources.font;
    let bold = resources.bold_font;
    drop(label);
    drop(value);
    drop(parent);
    drop(resources);
    assert!(!is_font_alive(regular));
    assert!(!is_font_alive(bold));
}
