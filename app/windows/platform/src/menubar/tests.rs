use super::*;
use crate::{
    controls::{set_window_text, window_text},
    search::SearchBox,
};
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());
const SEARCH: u16 = 119;
const ENABLED: usize = 502;
const DISABLED: usize = 501;
#[derive(Default)]
struct Events {
    changes: usize,
    last_edit: HWND,
    commands: Vec<usize>,
    popups: usize,
    select_popup: bool,
    switch_once: bool,
}
unsafe extern "system" fn fixture_proc(window: HWND, message: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        if message == WM_NCCREATE {
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                (*(lp as *const CREATESTRUCTW)).lpCreateParams as _,
            );
            return 1;
        }
        let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Events;
        if !state.is_null() {
            match message {
                WM_COMMAND
                    if (wp & 0xffff) == SEARCH as usize && (wp >> 16) == EN_CHANGE as usize =>
                {
                    (*state).changes += 1;
                    (*state).last_edit = lp as HWND;
                    return 0;
                }
                WM_COMMAND if lp == 0 => {
                    (*state).commands.push(wp & 0xffff);
                    return 0;
                }
                WM_INITMENUPOPUP => {
                    (*state).popups += 1;
                }
                WM_ENTERMENULOOP if (*state).switch_once => {
                    (*state).switch_once = false;
                    PostMessageW(window, WM_KEYDOWN, VK_RIGHT as usize, 0);
                }
                WM_ENTERMENULOOP if (*state).select_popup => {
                    PostMessageW(window, WM_KEYDOWN, VK_DOWN as usize, 0);
                    PostMessageW(window, WM_KEYDOWN, VK_RETURN as usize, 0);
                }
                WM_TIMER => {
                    EndMenu();
                }
                WM_NCDESTROY => {
                    SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                }
                _ => {}
            }
        }
        DefWindowProcW(window, message, wp, lp)
    }
}
struct Fixture {
    parent: HWND,
    menu: HMENU,
    events: Box<Events>,
}
impl Fixture {
    unsafe fn new(valid: bool) -> Self {
        unsafe {
            let menu = CreateMenu();
            for (index, title) in ["&File", "&Edit", "&View", "&Tools", "&Message", "&Help"]
                .into_iter()
                .enumerate()
            {
                let popup = CreatePopupMenu();
                AppendMenuW(
                    popup,
                    MF_STRING | MF_GRAYED,
                    DISABLED,
                    wide("Unavailable").as_ptr(),
                );
                AppendMenuW(popup, MF_STRING, ENABLED + index, wide("Execute").as_ptr());
                AppendMenuW(menu, MF_POPUP, popup as usize, wide(title).as_ptr());
            }
            if !valid {
                AppendMenuW(menu, MF_STRING, 599, wide("Not a popup").as_ptr());
            }
            let class = wide("WinMailMenuBarFixture");
            let definition = WNDCLASSW {
                lpfnWndProc: Some(fixture_proc),
                hInstance: GetModuleHandleW(null()),
                lpszClassName: class.as_ptr(),
                ..std::mem::zeroed()
            };
            if RegisterClassW(&definition) == 0 {
                assert_eq!(GetLastError(), ERROR_CLASS_ALREADY_EXISTS);
            }
            let mut events = Box::<Events>::default();
            let parent = CreateWindowExW(
                0,
                class.as_ptr(),
                wide("Menu fixture").as_ptr(),
                WS_OVERLAPPEDWINDOW,
                -10000,
                -10000,
                760,
                200,
                null_mut(),
                menu,
                GetModuleHandleW(null()),
                (&mut *events as *mut Events).cast(),
            );
            assert!(!parent.is_null());
            Self {
                parent,
                menu,
                events,
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
unsafe fn pump() {
    unsafe {
        let mut message = std::mem::zeroed();
        while PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}
fn key(window: HWND, message: u32, key: u16) -> MSG {
    MSG {
        hwnd: window,
        message,
        wParam: key as usize,
        ..unsafe { std::mem::zeroed() }
    }
}

#[test]
fn transfers_menu_ownership_preserves_disabled_commands_and_forwards_visible_search() {
    let _lock = LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    unsafe {
        let fixture = Fixture::new(true);
        let popup = GetSubMenu(fixture.menu, 0);
        let bar = MenuBar::create(fixture.parent).unwrap();
        assert!(GetMenu(fixture.parent).is_null());
        assert_eq!(bar.menu(), fixture.menu);
        assert_eq!(GetMenuItemCount(bar.menu()), 6);
        assert_ne!(
            GetMenuState(bar.menu(), DISABLED as u32, MF_BYCOMMAND) & MF_GRAYED,
            0
        );
        let search = SearchBox::create(bar.hwnd(), SEARCH).unwrap();
        MoveWindow(bar.hwnd(), 0, 0, 740, 30, 1);
        MoveWindow(search.hwnd(), 560, 2, 170, 26, 1);
        ShowWindow(fixture.parent, SW_SHOWNA);
        assert_ne!(IsWindowVisible(bar.hwnd()), 0);
        assert_ne!(IsWindowVisible(search.edit()), 0);
        set_window_text(search.edit(), "invoice");
        assert_eq!(fixture.events.changes, 1);
        assert_eq!(fixture.events.last_edit, search.edit());
        assert_eq!(window_text(search.edit()), "invoice");
        let button = GetDlgItem(bar.hwnd(), FIRST_BUTTON as i32);
        assert_eq!(window_text(button), "&File");
        for value in [96, 144, 192, 288, 384] {
            bar.update_dpi(value);
            MoveWindow(
                bar.hwnd(),
                0,
                0,
                dpi::scale(740, value),
                dpi::scale(30, value),
                1,
            );
            let mut previous_right = i32::MIN;
            for section in &lookup(bar.hwnd()).unwrap().sections {
                let mut bounds = RECT::default();
                GetWindowRect(section.button.get(), &mut bounds);
                assert!(bounds.left >= previous_right);
                assert_eq!(bounds.bottom - bounds.top, dpi::scale(30, value) - 2);
                previous_right = bounds.right;
            }
            assert!(bar.buttons_width() <= dpi::scale(500, value));
        }
        let menu = fixture.menu;
        drop(fixture);
        assert_eq!(
            IsMenu(menu),
            0,
            "Detached menu must be destroyed with its client row"
        );
        assert_eq!(
            IsMenu(popup),
            0,
            "Destroying the tree includes its borrowed popups"
        );
    }
}
#[test]
fn unsupported_menu_keeps_native_ownership_and_keyboard_navigation_opens_native_popup() {
    let _lock = LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    unsafe {
        let invalid = Fixture::new(false);
        assert!(MenuBar::create(invalid.parent).is_err());
        assert_eq!(GetMenu(invalid.parent), invalid.menu);
        assert_ne!(IsMenu(invalid.menu), 0);
        drop(invalid);
        let mut fixture = Fixture::new(true);
        let bar = MenuBar::create(fixture.parent).unwrap();
        let edit = CreateWindowExW(
            0,
            wide("EDIT").as_ptr(),
            wide("").as_ptr(),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            0,
            40,
            200,
            24,
            fixture.parent,
            150usize as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        ShowWindow(fixture.parent, SW_SHOWNA);
        SetFocus(edit);
        let mut original = [0u8; 256];
        assert_ne!(GetKeyboardState(original.as_mut_ptr()), 0);
        let mut modified = original;
        modified[VK_SHIFT as usize] = 0x80;
        assert_ne!(SetKeyboardState(modified.as_ptr()), 0);
        assert!(
            !bar.translate(&key(edit, WM_KEYDOWN, VK_F10)),
            "Shift+F10 must keep the native context-menu route"
        );
        modified = original;
        modified[VK_CONTROL as usize] = 0x80;
        assert_ne!(SetKeyboardState(modified.as_ptr()), 0);
        assert!(
            !bar.translate(&key(edit, WM_SYSKEYDOWN, b'F' as u16)),
            "AltGr/Ctrl+Alt must not open a menu"
        );
        assert_ne!(SetKeyboardState(original.as_ptr()), 0);
        assert!(bar.translate(&key(edit, WM_KEYDOWN, VK_F10)));
        assert_eq!(GetFocus(), GetDlgItem(bar.hwnd(), FIRST_BUTTON as i32));
        assert!(bar.translate(&key(GetFocus(), WM_KEYDOWN, VK_RIGHT)));
        assert_eq!(GetFocus(), GetDlgItem(bar.hwnd(), FIRST_BUTTON as i32 + 1));
        assert!(bar.translate(&key(GetFocus(), WM_KEYDOWN, VK_ESCAPE)));
        assert_eq!(GetFocus(), edit);
        assert!(bar.translate(&key(edit, WM_SYSKEYDOWN, VK_MENU)));
        assert!(bar.translate(&key(edit, WM_SYSKEYUP, VK_MENU)));
        assert_eq!(GetFocus(), GetDlgItem(bar.hwnd(), FIRST_BUTTON as i32));
        assert!(bar.translate(&key(GetFocus(), WM_KEYDOWN, VK_ESCAPE)));
        fixture.events.select_popup = true;
        SetTimer(fixture.parent, 15, 5000, None);
        assert!(bar.translate(&key(edit, WM_SYSKEYDOWN, b'F' as u16)));
        pump();
        KillTimer(fixture.parent, 15);
        assert!(
            fixture.events.popups >= 1,
            "Alt+F must open the original native popup"
        );
        assert_eq!(
            fixture.events.commands,
            vec![ENABLED],
            "Disabled commands must not execute or duplicate"
        );
        assert_eq!(GetFocus(), edit);
        assert!(!lookup(bar.hwnd()).unwrap().popup_open.get());
        fixture.events.commands.clear();
        fixture.events.switch_once = true;
        let before = fixture.events.popups;
        SetTimer(fixture.parent, 15, 5000, None);
        assert!(bar.translate(&key(edit, WM_SYSKEYDOWN, b'F' as u16)));
        pump();
        KillTimer(fixture.parent, 15);
        assert!(
            fixture.events.popups >= before + 2,
            "Right arrow must switch native top-level popups"
        );
        assert_eq!(fixture.events.commands, vec![ENABLED + 1]);
        assert_eq!(GetFocus(), edit);
    }
}
