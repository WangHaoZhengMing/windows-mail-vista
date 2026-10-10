//! Native dialog layout and window-class registration shared by settings and accounts.
use crate::{
    Result,
    platform::{controls::account_control as child, dpi},
};
use std::ptr::{null, null_mut};
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};

struct ControlBounds {
    window: HWND,
    rect: [i32; 4],
}

#[derive(Default)]
pub(crate) struct WindowLayout {
    pub(crate) metrics: dpi::Resources,
    bounds: Vec<ControlBounds>,
}

impl WindowLayout {
    pub(crate) unsafe fn control(
        &mut self,
        owner: HWND,
        class: &str,
        caption: &str,
        style: u32,
        id: u16,
        rect: [i32; 4],
    ) -> HWND {
        unsafe {
            let window = child(owner, class, caption, style, id);
            self.bounds.push(ControlBounds { window, rect });
            let [x, y, width, height] = rect;
            dpi::move_control(self.metrics.dpi, window, x, y, width, height, 0);
            window
        }
    }

    pub(crate) unsafe fn update(&mut self, window: HWND, value: u32) {
        unsafe {
            self.metrics.update(window, value);
            for bounds in &self.bounds {
                let [x, y, width, height] = bounds.rect;
                dpi::move_control(self.metrics.dpi, bounds.window, x, y, width, height, 1);
            }
        }
    }
}

pub(crate) unsafe fn register_class(class: &[u16], procedure: WNDPROC) -> Result<()> {
    unsafe {
        let definition = WNDCLASSW {
            lpfnWndProc: procedure,
            hInstance: GetModuleHandleW(null()),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: (COLOR_WINDOW + 1) as HBRUSH,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        if RegisterClassW(&definition) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(format!(
                "Could not register account window: {}",
                GetLastError()
            ));
        }
        Ok(())
    }
}
