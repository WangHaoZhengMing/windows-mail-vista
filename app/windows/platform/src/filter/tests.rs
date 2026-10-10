use super::super::{controls::control, strings::wide};
use super::*;
use std::sync::{Mutex, OnceLock};
use windows_sys::Win32::{
    System::LibraryLoader::GetModuleHandleW,
    UI::Input::KeyboardAndMouse::{SetFocus, VK_DOWN},
};

static LOCK: Mutex<()> = Mutex::new(());
#[derive(Default)]
struct Notices {
    changed: usize,
    dropdown: usize,
    closed: usize,
    source: HWND,
}
unsafe extern "system" fn owner_proc(window: HWND, message: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        if message == WM_NCCREATE {
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                (*(lp as *const CREATESTRUCTW)).lpCreateParams as isize,
            );
            return 1;
        }
        if message == WM_COMMAND {
            let notices = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Notices;
            if !notices.is_null() {
                match (wp >> 16) as u32 {
                    CBN_SELCHANGE => (*notices).changed += 1,
                    CBN_DROPDOWN => (*notices).dropdown += 1,
                    CBN_CLOSEUP => (*notices).closed += 1,
                    _ => (),
                }
                (*notices).source = lp as HWND;
            }
        }
        DefWindowProcW(window, message, wp, lp)
    }
}
struct Fixture {
    owner: HWND,
    combo: HWND,
    notices: Box<Notices>,
    resources: dpi::Resources,
}
impl Fixture {
    unsafe fn new() -> Self {
        unsafe {
            static CLASS: OnceLock<()> = OnceLock::new();
            CLASS.get_or_init(|| {
                let name = wide("WinMail.Filter.Test");
                let class = WNDCLASSW {
                    lpfnWndProc: Some(owner_proc),
                    hInstance: GetModuleHandleW(null()),
                    lpszClassName: name.as_ptr(),
                    ..Default::default()
                };
                assert_ne!(RegisterClassW(&class), 0);
            });
            let mut notices = Box::<Notices>::default();
            let owner = CreateWindowExW(
                0,
                wide("WinMail.Filter.Test").as_ptr(),
                wide("").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                -32000,
                -32000,
                400,
                300,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                (&mut *notices as *mut Notices).cast(),
            );
            assert!(!owner.is_null());
            let combo = control(
                owner,
                "COMBOBOX",
                "",
                CBS_DROPDOWNLIST as u32 | WS_TABSTOP,
                120,
            );
            for label in ["All messages", "Unread messages", "Read messages"] {
                SendMessageW(combo, CB_ADDSTRING, 0, wide(label).as_ptr() as isize);
            }
            SendMessageW(combo, CB_SETCURSEL, 0, 0);
            let mut resources = dpi::Resources::default();
            resources.update(owner, 96);
            MoveWindow(combo, 0, 0, 174, 190, 0);
            assert!(style_filter(combo));
            assert!(style_filter(combo));
            Self {
                owner,
                combo,
                notices,
                resources,
            }
        }
    }
    unsafe fn rect(&self) -> RECT {
        unsafe {
            let mut rect = RECT::default();
            GetClientRect(self.combo, &mut rect);
            rect
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.owner);
        }
    }
}
struct Canvas {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
}
impl Canvas {
    unsafe fn new(width: i32, height: i32) -> Self {
        unsafe {
            let screen = GetDC(null_mut());
            let dc = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, width, height);
            ReleaseDC(null_mut(), screen);
            let old = SelectObject(dc, bitmap);
            Self { dc, bitmap, old }
        }
    }
    unsafe fn white_pixels(&self, rect: RECT) -> usize {
        unsafe {
            (rect.top..rect.bottom)
                .flat_map(|y| (rect.left..rect.right).map(move |x| (x, y)))
                .filter(|&(x, y)| GetPixel(self.dc, x, y) == WHITE)
                .count()
        }
    }
}
impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}
unsafe fn print(combo: HWND, canvas: &Canvas) {
    unsafe {
        SendMessageW(
            combo,
            WM_PRINTCLIENT,
            canvas.dc as usize,
            PRF_CLIENT as isize,
        );
    }
}

#[test]
fn selection_keyboard_notifications_and_popup_remain_native() {
    let _guard = LOCK.lock().unwrap();
    unsafe {
        let fixture = Fixture::new();
        assert_eq!(SendMessageW(fixture.combo, CB_GETCOUNT, 0, 0), 3);
        assert_eq!(SendMessageW(fixture.combo, CB_SETCURSEL, 2, 0), 2);
        assert_eq!(selected_caption(fixture.combo), wide("Read messages"));
        assert_eq!(
            fixture.notices.changed, 0,
            "Programmatic selection must not invent a CBN_SELCHANGE"
        );
        SendMessageW(fixture.combo, CB_SETCURSEL, 0, 0);
        SetFocus(fixture.combo);
        SendMessageW(fixture.combo, WM_KEYDOWN, VK_DOWN as usize, 0);
        assert_eq!(SendMessageW(fixture.combo, CB_GETCURSEL, 0, 0), 1);
        assert_eq!(fixture.notices.changed, 1);
        assert_eq!(fixture.notices.source, fixture.combo);
        let mut info = COMBOBOXINFO {
            cbSize: size_of::<COMBOBOXINFO>() as u32,
            ..Default::default()
        };
        assert_ne!(GetComboBoxInfo(fixture.combo, &mut info), 0);
        assert_ne!(IsWindow(info.hwndList), 0);
        let mut class = [0u16; 64];
        let copied = GetClassNameW(info.hwndList, class.as_mut_ptr(), class.len() as i32);
        assert!(
            String::from_utf16_lossy(&class[..copied as usize]).eq_ignore_ascii_case("ComboLBox"),
            "The popup remains the system ComboLBox rather than a custom list"
        );
        SendMessageW(fixture.combo, CB_SHOWDROPDOWN, 1, 0);
        assert_ne!(SendMessageW(fixture.combo, CB_GETDROPPEDSTATE, 0, 0), 0);
        assert_eq!(fixture.notices.dropdown, 1);
        SendMessageW(fixture.combo, CB_SHOWDROPDOWN, 0, 0);
        assert_eq!(SendMessageW(fixture.combo, CB_GETDROPPEDSTATE, 0, 0), 0);
        assert_eq!(fixture.notices.closed, 1);
        assert_eq!(SendMessageW(fixture.combo, CB_GETCURSEL, 0, 0), 1);
        assert!(!style_filter(fixture.owner));
    }
}

#[test]
fn printed_face_keeps_white_caption_arrow_and_focus_at_font_and_window_scales() {
    let _guard = LOCK.lock().unwrap();
    unsafe {
        let mut fixture = Fixture::new();
        for scale in [96, 192, 288, 384] {
            fixture.resources.update(fixture.owner, scale);
            MoveWindow(
                fixture.combo,
                0,
                0,
                dpi::scale(174, scale),
                dpi::scale(190, scale),
                0,
            );
            let rect = fixture.rect();
            let canvas = Canvas::new(rect.right, rect.bottom);
            let old_font = GetCurrentObject(canvas.dc, OBJ_FONT as u32);
            print(fixture.combo, &canvas);
            assert_eq!(
                GetCurrentObject(canvas.dc, OBJ_FONT as u32),
                old_font,
                "Painting must restore the caller's HDC font"
            );
            assert_eq!(GetPixel(canvas.dc, rect.right / 2, 0), BORDER);
            let top = GetPixel(canvas.dc, rect.right / 2, 2);
            let bottom = GetPixel(canvas.dc, rect.right / 2, rect.bottom - 3);
            assert_ne!(
                top, bottom,
                "The closed face must retain the toolbar gradient"
            );
            assert!(top >> 16 & 255 > top & 255, "The background must be blue");
            let mut info = COMBOBOXINFO {
                cbSize: size_of::<COMBOBOXINFO>() as u32,
                ..Default::default()
            };
            GetComboBoxInfo(fixture.combo, &mut info);
            assert!(
                canvas.white_pixels(RECT {
                    left: 2,
                    top: 2,
                    right: info.rcButton.left - 2,
                    bottom: rect.bottom - 2
                }) > 10,
                "Selected caption must draw white glyphs using the current font"
            );
            assert!(
                canvas.white_pixels(RECT {
                    left: info.rcButton.left + 1,
                    top: 2,
                    right: info.rcButton.right - 1,
                    bottom: rect.bottom - 2
                }) > 3,
                "The native button area must contain the white down arrow"
            );
            SetFocus(fixture.combo);
            print(fixture.combo, &canvas);
            assert_eq!(
                GetPixel(canvas.dc, rect.right / 2, 0),
                WHITE,
                "Keyboard focus needs a visible outline"
            );
            SetFocus(fixture.owner);
        }
        let rect = fixture.rect();
        let canvas = Canvas::new(rect.right, rect.bottom);
        for selected in [0, 1, 2, 0] {
            SendMessageW(fixture.combo, CB_SETCURSEL, selected, 0);
            print(fixture.combo, &canvas);
            assert_eq!(
                SendMessageW(fixture.combo, CB_GETCURSEL, 0, 0),
                selected as isize
            );
        }
    }
}
