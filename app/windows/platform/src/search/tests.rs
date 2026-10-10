use super::super::controls::{set_window_text, window_text};
use super::*;
use std::sync::Mutex;
use windows_sys::Win32::{
    Foundation::{HANDLE, INVALID_HANDLE_VALUE},
    System::{
        ApplicationInstallationAndServicing::{
            ACTCTXW, ActivateActCtx, CreateActCtxW, DeactivateActCtx, ReleaseActCtx,
        },
        Threading::{GR_GDIOBJECTS, GetCurrentProcess, GetGuiResources},
    },
};

static LOCK: Mutex<()> = Mutex::new(());
const SEARCH_ID: u16 = 119;
// Cargo's test executable has no Win32 manifest. Match the application's visual
// styles so EDIT's native cue banner and themed keyboard behavior are exercised.
struct VisualStyles {
    context: HANDLE,
    cookie: usize,
}
impl VisualStyles {
    unsafe fn enter() -> Self {
        unsafe {
            let manifest = wide(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../assets/windows/manifests/app.manifest"
            ));
            let definition = ACTCTXW {
                cbSize: std::mem::size_of::<ACTCTXW>() as u32,
                lpSource: manifest.as_ptr(),
                ..std::mem::zeroed()
            };
            let context = CreateActCtxW(&definition);
            assert_ne!(
                context,
                INVALID_HANDLE_VALUE,
                "Could not activate app manifest: {}",
                GetLastError()
            );
            let mut cookie = 0;
            assert_ne!(ActivateActCtx(context, &mut cookie), 0);
            Self { context, cookie }
        }
    }
}
impl Drop for VisualStyles {
    fn drop(&mut self) {
        unsafe {
            DeactivateActCtx(0, self.cookie);
            ReleaseActCtx(self.context);
        }
    }
}
#[derive(Default)]
struct Notifications {
    changes: usize,
    last: HWND,
}
unsafe extern "system" fn parent_proc(window: HWND, msg: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        if msg == WM_NCCREATE {
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                (*(lp as *const CREATESTRUCTW)).lpCreateParams as _,
            );
            return 1;
        }
        if msg == WM_COMMAND
            && (wp & 0xffff) as u16 == SEARCH_ID
            && (wp >> 16) as u16 == EN_CHANGE as u16
        {
            let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Notifications;
            if !state.is_null() {
                (*state).changes += 1;
                (*state).last = lp as HWND;
            }
            return 0;
        }
        if msg == WM_NCDESTROY {
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
        }
        DefWindowProcW(window, msg, wp, lp)
    }
}
struct Fixture {
    parent: HWND,
    search: SearchBox,
    after: HWND,
    notifications: Box<Notifications>,
}
impl Fixture {
    unsafe fn new() -> Self {
        unsafe {
            let class = wide("WinMailSearchControlTest");
            let definition = WNDCLASSW {
                lpfnWndProc: Some(parent_proc),
                hInstance: GetModuleHandleW(null()),
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            if RegisterClassW(&definition) == 0 {
                assert_eq!(GetLastError(), ERROR_CLASS_ALREADY_EXISTS);
            }
            let mut notifications = Box::<Notifications>::default();
            let parent = CreateWindowExW(
                WS_EX_CONTROLPARENT,
                class.as_ptr(),
                wide("Search fixture").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                450,
                180,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                (&mut *notifications as *mut Notifications).cast(),
            );
            assert!(!parent.is_null());
            let search = SearchBox::create(parent, SEARCH_ID).unwrap();
            MoveWindow(search.hwnd(), 8, 8, 170, 26, 1);
            let after = CreateWindowExW(
                0,
                wide("BUTTON").as_ptr(),
                wide("Next").as_ptr(),
                WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                190,
                8,
                80,
                26,
                parent,
                120usize as HMENU,
                GetModuleHandleW(null()),
                null(),
            );
            Self {
                parent,
                search,
                after,
                notifications,
            }
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.parent);
        }
    }
}
#[test]
fn real_edit_forwards_changes_clear_escape_enter_tab_and_scales_layout() {
    let _lock = LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    unsafe {
        let _styles = VisualStyles::enter();
        let fixture = Fixture::new();
        let edit = fixture.search.edit();
        assert_eq!(GetDlgCtrlID(edit), SEARCH_ID as i32);
        let mut cue = [0u16; 32];
        assert_ne!(
            SendMessageW(
                edit,
                EM_GETCUEBANNER,
                cue.as_mut_ptr() as usize,
                cue.len() as isize
            ),
            0
        );
        assert_eq!(String::from_utf16_lossy(&cue[..11]), "Search mail");
        set_window_text(edit, "invoice");
        assert_eq!(fixture.notifications.changes, 1);
        assert_eq!(fixture.notifications.last, edit);
        let button = GetDlgItem(fixture.search.hwnd(), ICON_BUTTON as i32);
        assert_eq!(window_text(button), "Clear search");
        SendMessageW(button, BM_CLICK, 0, 0);
        assert_eq!(window_text(edit), "");
        assert_eq!(fixture.notifications.changes, 2);
        assert_eq!(GetFocus(), edit);
        assert_eq!(window_text(button), "Search mail");
        SendMessageW(edit, WM_CHAR, 'x' as usize, 0);
        assert_eq!(window_text(edit), "x");
        SendMessageW(edit, WM_KEYDOWN, VK_RETURN as usize, 0);
        assert_eq!(fixture.notifications.changes, 4);
        SendMessageW(edit, WM_KEYDOWN, VK_ESCAPE as usize, 0);
        assert_eq!(window_text(edit), "");
        assert_eq!(fixture.notifications.changes, 5);
        SendMessageW(edit, WM_CHAR, VK_RETURN as usize, 0);
        SendMessageW(edit, WM_CHAR, VK_ESCAPE as usize, 0);
        assert_eq!(window_text(edit), "");
        assert_eq!(fixture.notifications.changes, 5);
        let tab = MSG {
            hwnd: edit,
            message: WM_KEYDOWN,
            wParam: VK_TAB as usize,
            ..std::mem::zeroed()
        };
        assert_ne!(IsDialogMessageW(fixture.parent, &tab), 0);
        assert_eq!(
            GetFocus(),
            fixture.after,
            "Tab must escape the composite host"
        );
        for value in [96, 144, 192, 288, 384, 96] {
            fixture.search.update_dpi(value);
            MoveWindow(
                fixture.search.hwnd(),
                0,
                0,
                dpi::scale(170, value),
                dpi::scale(26, value),
                1,
            );
            let mut outer = RECT::default();
            let mut input = RECT::default();
            let mut icon = RECT::default();
            GetClientRect(fixture.search.hwnd(), &mut outer);
            GetWindowRect(edit, &mut input);
            GetWindowRect(button, &mut icon);
            assert_eq!(outer.bottom, dpi::scale(26, value));
            assert_eq!(icon.right - icon.left, dpi::scale(22, value));
            assert!(input.right < icon.left);
            assert!(input.bottom - input.top <= outer.bottom - dpi::scale(4, value));
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
#[test]
fn white_frame_focus_border_and_vector_painting_release_gdi_objects() {
    let _lock = LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    unsafe {
        let _styles = VisualStyles::enter();
        let fixture = Fixture::new();
        let canvas = Canvas::new(170, 26);
        SendMessageW(
            fixture.search.hwnd(),
            WM_PRINTCLIENT,
            canvas.dc as usize,
            PRF_CLIENT as isize,
        );
        assert_eq!(
            GetPixel(canvas.dc, 25, 12),
            WHITE,
            "Edit background is white, not toolbar blue"
        );
        let idle = GetPixel(canvas.dc, 80, 0);
        SetFocus(fixture.search.edit());
        SendMessageW(
            fixture.search.hwnd(),
            WM_PRINTCLIENT,
            canvas.dc as usize,
            PRF_CLIENT as isize,
        );
        assert_ne!(
            GetPixel(canvas.dc, 80, 0),
            idle,
            "Focused border has a distinct blue glow"
        );
        let button = GetDlgItem(fixture.search.hwnd(), ICON_BUTTON as i32);
        let mut draw = DRAWITEMSTRUCT {
            CtlType: ODT_BUTTON,
            CtlID: ICON_BUTTON as u32,
            hwndItem: button,
            hDC: canvas.dc,
            rcItem: RECT {
                left: 0,
                top: 0,
                right: 22,
                bottom: 22,
            },
            ..std::mem::zeroed()
        };
        SendMessageW(
            fixture.search.hwnd(),
            WM_DRAWITEM,
            ICON_BUTTON as usize,
            &mut draw as *mut _ as isize,
        );
        let before = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
        for _ in 0..200 {
            SendMessageW(
                fixture.search.hwnd(),
                WM_PRINTCLIENT,
                canvas.dc as usize,
                PRF_CLIENT as isize,
            );
            SendMessageW(
                fixture.search.hwnd(),
                WM_DRAWITEM,
                ICON_BUTTON as usize,
                &mut draw as *mut _ as isize,
            );
        }
        let after = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
        assert!(
            after <= before + 16,
            "Repeated painting leaked GDI resources: {before} -> {after}"
        );
        drop(canvas);
        drop(fixture);
        let baseline = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
        for _ in 0..40 {
            let fixture = Fixture::new();
            drop(fixture);
        }
        let final_count = GetGuiResources(GetCurrentProcess(), GR_GDIOBJECTS);
        assert!(
            final_count <= baseline + 24,
            "Destroying owner must dispose search children and resources: {baseline} -> {final_count}"
        );
    }
}
