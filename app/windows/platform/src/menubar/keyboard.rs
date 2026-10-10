//! Menu-bar navigation around Windows' native modal popup tracking.
use super::*;
thread_local! { static TRACKING: Cell<HWND> = const { Cell::new(null_mut()) }; }

fn mnemonic(label: &[u16]) -> Option<u16> {
    let mut index = 0;
    while index + 1 < label.len() {
        if label[index] == '&' as u16 {
            if label[index + 1] != '&' as u16 {
                return Some(uppercase(label[index + 1]));
            }
            index += 1;
        }
        index += 1;
    }
    None
}
fn uppercase(value: u16) -> u16 {
    if (b'a' as u16..=b'z' as u16).contains(&value) {
        value - 32
    } else {
        value
    }
}

pub(super) unsafe fn translate(window: HWND, message: &MSG) -> bool {
    unsafe {
        let Some(state) = lookup(window) else {
            return false;
        };
        if GetAncestor(message.hwnd, GA_ROOT) != state.parent {
            return false;
        }
        let key = message.wParam;
        // AltGr is Ctrl+Alt on Windows. Preserve text entry and shortcuts that
        // combine Ctrl with Alt rather than entering the menu mnemonic path.
        if matches!(message.message, WM_SYSKEYDOWN | WM_SYSKEYUP | WM_SYSCHAR)
            && GetKeyState(VK_CONTROL as i32) < 0
        {
            state.alt_down.set(false);
            return false;
        }
        if message.message == WM_KEYDOWN && key == VK_CONTROL as usize {
            state.alt_down.set(false);
        }
        match message.message {
            WM_SYSKEYDOWN if key == VK_MENU as usize => {
                state.alt_down.set(true);
                true
            }
            WM_SYSKEYUP if key == VK_MENU as usize => {
                if state.alt_down.replace(false) {
                    if state.keyboard.get() {
                        restore_focus(window);
                    } else {
                        state.focus(window, 0);
                    }
                }
                true
            }
            WM_SYSKEYDOWN | WM_SYSCHAR if key != VK_MENU as usize => {
                state.alt_down.set(false);
                if let Some(index) = state
                    .sections
                    .iter()
                    .position(|section| mnemonic(&section.label) == Some(uppercase(key as u16)))
                {
                    state.focus(window, index);
                    PostMessageW(window, OPEN, index, 0);
                    return true;
                }
                false
            }
            WM_KEYDOWN if key == VK_F10 as usize && GetKeyState(VK_SHIFT as i32) >= 0 => {
                if state.keyboard.get() {
                    restore_focus(window);
                } else {
                    state.focus(window, 0);
                }
                true
            }
            WM_KEYUP if key == VK_F10 as usize && GetKeyState(VK_SHIFT as i32) >= 0 => true,
            WM_KEYDOWN if state.keyboard.get() => {
                button_key(window, state.active.get().unwrap_or(0), key)
            }
            _ => false,
        }
    }
}
pub(super) unsafe fn button_key(window: HWND, index: usize, key: usize) -> bool {
    unsafe {
        let Some(state) = lookup(window) else {
            return false;
        };
        let count = state.sections.len();
        match key as u16 {
            VK_LEFT => state.focus(window, (index + count - 1) % count),
            VK_RIGHT => state.focus(window, (index + 1) % count),
            VK_DOWN | VK_UP | VK_RETURN | VK_SPACE => {
                PostMessageW(window, OPEN, index, 0);
            }
            VK_ESCAPE | VK_TAB => restore_focus(window),
            _ => return false,
        }
        true
    }
}
/// A thread-local hook exists only while TrackPopupMenuEx owns its native loop.
/// It switches top-level headings; nested submenu arrows stay native Windows.
pub(super) struct PopupHook {
    hook: HHOOK,
    previous: HWND,
}
impl PopupHook {
    pub(super) unsafe fn new(window: HWND) -> Self {
        unsafe {
            let previous = TRACKING.with(|tracking| tracking.replace(window));
            let hook = SetWindowsHookExW(
                WH_MSGFILTER,
                Some(menu_filter),
                null_mut(),
                GetCurrentThreadId(),
            );
            Self { hook, previous }
        }
    }
}
impl Drop for PopupHook {
    fn drop(&mut self) {
        unsafe {
            if !self.hook.is_null() {
                UnhookWindowsHookEx(self.hook);
            }
            TRACKING.with(|tracking| tracking.set(self.previous));
        }
    }
}
unsafe extern "system" fn menu_filter(code: i32, wp: usize, lp: isize) -> isize {
    unsafe {
        if code == MSGF_MENU as i32 {
            let window = TRACKING.with(Cell::get);
            if let Some(state) = lookup(window) {
                let message = &*(lp as *const MSG);
                let active = state.active.get().unwrap_or(0);
                let mut next = None;
                if message.message == WM_KEYDOWN
                    && state.selected_popup.get() == state.sections[active].popup
                {
                    if message.wParam == VK_LEFT as usize {
                        next = Some((active + state.sections.len() - 1) % state.sections.len());
                    }
                    if message.wParam == VK_RIGHT as usize && !state.selected_has_children.get() {
                        next = Some((active + 1) % state.sections.len());
                    }
                }
                if message.message == WM_MOUSEMOVE {
                    let mut cursor = POINT::default();
                    GetCursorPos(&mut cursor);
                    next = state
                        .sections
                        .iter()
                        .position(|section| {
                            let mut bounds = RECT::default();
                            GetWindowRect(section.button.get(), &mut bounds);
                            cursor.x >= bounds.left
                                && cursor.x < bounds.right
                                && cursor.y >= bounds.top
                                && cursor.y < bounds.bottom
                        })
                        .filter(|&index| index != active);
                }
                if let Some(next) = next {
                    state.pending.set(Some(next));
                    EndMenu();
                    return 1;
                }
            }
        }
        CallNextHookEx(null_mut(), code, wp, lp)
    }
}
