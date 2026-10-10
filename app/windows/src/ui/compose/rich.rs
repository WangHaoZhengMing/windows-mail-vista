//! Native RichEdit formatting and portable HTML/RTF serialization.
use super::*;
pub(super) use windows::Win32::UI::Controls::RichEdit::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{EnableWindow, SetFocus};

pub const FIRST: u16 = 450;
pub unsafe fn selection(body: HWND) -> CHARRANGE {
    let mut range = CHARRANGE::default();
    unsafe {
        SendMessageW(body, EM_EXGETSEL, 0, &mut range as *mut _ as isize);
    }
    range
}
pub unsafe fn select(body: HWND, range: CHARRANGE) {
    unsafe {
        SendMessageW(body, EM_EXSETSEL, 0, &range as *const _ as isize);
    }
}
pub unsafe fn char_format(body: HWND) -> CHARFORMATW {
    let mut format = CHARFORMATW {
        cbSize: std::mem::size_of::<CHARFORMATW>() as u32,
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            body,
            EM_GETCHARFORMAT,
            SCF_SELECTION as usize,
            &mut format as *mut _ as isize,
        );
    }
    format
}
pub unsafe fn apply(body: HWND, format: &CHARFORMATW) {
    unsafe {
        SendMessageW(
            body,
            EM_SETCHARFORMAT,
            SCF_SELECTION as usize,
            format as *const _ as isize,
        );
    }
}
pub unsafe fn initialize(state: &ComposeState) {
    unsafe {
        let mut format = CHARFORMATW {
            cbSize: std::mem::size_of::<CHARFORMATW>() as u32,
            dwMask: CFM_FACE | CFM_SIZE,
            yHeight: 240,
            ..Default::default()
        };
        format.szFaceName[..6].copy_from_slice(&wide("Arial"));
        SendMessageW(
            state.body,
            EM_SETCHARFORMAT,
            0,
            &format as *const _ as isize,
        );
        SendMessageW(
            state.body,
            EM_SETEVENTMASK,
            0,
            (ENM_CHANGE | ENM_SELCHANGE) as isize,
        );
        SendMessageW(state.body, EM_SETBKGNDCOLOR, 0, 0xffffff);
    }
}
pub unsafe fn sync(state: &ComposeState) {
    if state.format.len() != 8 {
        return;
    }
    unsafe {
        let format = char_format(state.body);
        for (index, effect) in [(2, CFE_BOLD), (3, CFE_ITALIC), (4, CFE_UNDERLINE)] {
            SendMessageW(
                state.format[index],
                BM_SETCHECK,
                (format.dwEffects.0 & effect.0 != 0) as usize,
                0,
            );
        }
        let face = String::from_utf16_lossy(
            &format.szFaceName[..format.szFaceName.iter().position(|c| *c == 0).unwrap_or(32)],
        );
        for (index, value) in [(0, face), (1, (format.yHeight / 20).to_string())] {
            let selected = SendMessageW(
                state.format[index],
                CB_FINDSTRINGEXACT,
                usize::MAX,
                wide(&value).as_ptr() as isize,
            );
            SendMessageW(state.format[index], CB_SETCURSEL, selected as usize, 0);
        }
        for control in &state.format {
            EnableWindow(*control, state.rich.get() as i32);
        }
    }
}
pub unsafe fn command(state: &ComposeState, index: usize) {
    unsafe {
        let mut format = char_format(state.body);
        match index {
            0 => {
                format.dwMask = CFM_FACE;
                format.szFaceName = [0; 32];
                for (slot, c) in format
                    .szFaceName
                    .iter_mut()
                    .take(31)
                    .zip(text(state.format[0]).encode_utf16())
                {
                    *slot = c;
                }
                apply(state.body, &format);
            }
            1 => {
                if let Ok(size) = text(state.format[1]).parse::<i32>() {
                    format.dwMask = CFM_SIZE;
                    format.yHeight = size.clamp(8, 72) * 20;
                    apply(state.body, &format);
                }
            }
            2..=4 => {
                let mask = [CFM_BOLD, CFM_ITALIC, CFM_UNDERLINE][index - 2];
                let effect = [CFE_BOLD, CFE_ITALIC, CFE_UNDERLINE][index - 2];
                format.dwMask = mask;
                format.dwEffects.0 ^= effect.0;
                apply(state.body, &format);
            }
            5 => {
                use windows_sys::Win32::UI::Controls::Dialogs::*;
                let mut colors = state.colors.borrow_mut();
                let mut choice = CHOOSECOLORW {
                    lStructSize: std::mem::size_of::<CHOOSECOLORW>() as u32,
                    hwndOwner: state.hwnd,
                    rgbResult: format.crTextColor.0,
                    lpCustColors: colors.as_mut_ptr(),
                    Flags: CC_FULLOPEN | CC_RGBINIT,
                    ..Default::default()
                };
                if ChooseColorW(&mut choice) != 0 {
                    format.dwMask = CFM_COLOR;
                    format.dwEffects.0 &= !CFE_AUTOCOLOR.0;
                    format.crTextColor.0 = choice.rgbResult;
                    apply(state.body, &format);
                } else {
                    return;
                }
            }
            6 | 7 => {
                let mut paragraph = PARAFORMAT {
                    cbSize: std::mem::size_of::<PARAFORMAT>() as u32,
                    ..Default::default()
                };
                SendMessageW(
                    state.body,
                    EM_GETPARAFORMAT,
                    0,
                    &mut paragraph as *mut _ as isize,
                );
                if index == 6 {
                    paragraph.dwMask = PFM_ALIGNMENT;
                    paragraph.wAlignment = match paragraph.wAlignment {
                        PFA_LEFT => PFA_CENTER,
                        PFA_CENTER => PFA_RIGHT,
                        _ => PFA_LEFT,
                    };
                } else {
                    paragraph.dwMask = PFM_NUMBERING | PFM_STARTINDENT | PFM_OFFSET;
                    paragraph.wNumbering = if paragraph.wNumbering == PFN_BULLET {
                        PARAFORMAT_NUMBERING(0)
                    } else {
                        PFN_BULLET
                    };
                    paragraph.dxStartIndent = if paragraph.wNumbering == PFN_BULLET {
                        360
                    } else {
                        0
                    };
                    paragraph.dxOffset = 180;
                }
                SendMessageW(
                    state.body,
                    EM_SETPARAFORMAT,
                    0,
                    &paragraph as *const _ as isize,
                );
            }
            _ => return,
        }
        state.dirty.set(true);
        SetFocus(state.body);
        sync(state);
    }
}
unsafe extern "system" fn output(
    cookie: usize,
    buffer: *mut u8,
    count: i32,
    written: *mut i32,
) -> u32 {
    unsafe {
        (*(cookie as *mut Vec<u8>))
            .extend_from_slice(std::slice::from_raw_parts(buffer, count as usize));
        *written = count;
    }
    0
}
unsafe extern "system" fn input(
    cookie: usize,
    buffer: *mut u8,
    count: i32,
    written: *mut i32,
) -> u32 {
    unsafe {
        let data = &mut *(cookie as *mut &[u8]);
        let count = data.len().min(count as usize);
        std::ptr::copy_nonoverlapping(data.as_ptr(), buffer, count);
        *data = &data[count..];
        *written = count as i32;
    }
    0
}
pub unsafe fn rtf(body: HWND) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut stream = EDITSTREAM {
        dwCookie: &mut bytes as *mut Vec<u8> as usize,
        pfnCallback: Some(output),
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            body,
            EM_STREAMOUT,
            SF_RTF as usize,
            &mut stream as *mut _ as isize,
        );
    }
    bytes
}
pub unsafe fn restore(body: HWND, mut bytes: &[u8]) {
    let mut stream = EDITSTREAM {
        dwCookie: &mut bytes as *mut &[u8] as usize,
        pfnCallback: Some(input),
        ..Default::default()
    };
    unsafe {
        SendMessageW(
            body,
            EM_STREAMIN,
            SF_RTF as usize,
            &mut stream as *mut _ as isize,
        );
    }
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
unsafe fn spans(body: HWND, units: &[u16], offset: usize, html: &mut String) {
    if units.is_empty() {
        return;
    }
    unsafe {
        select(
            body,
            CHARRANGE {
                cpMin: offset as i32,
                cpMax: (offset + units.len()) as i32,
            },
        );
        let format = char_format(body);
        let required = CFM_FACE | CFM_SIZE | CFM_BOLD | CFM_ITALIC | CFM_UNDERLINE | CFM_COLOR;
        if units.len() > 1 && format.dwMask.0 & required.0 != required.0 {
            let mut middle = units.len() / 2;
            if middle > 0
                && (0xd800..=0xdbff).contains(&units[middle - 1])
                && (0xdc00..=0xdfff).contains(&units[middle])
            {
                middle += 1;
            }
            if middle < units.len() {
                spans(body, &units[..middle], offset, html);
                spans(body, &units[middle..], offset + middle, html);
                return;
            }
        }
        let face = String::from_utf16_lossy(
            &format.szFaceName[..format.szFaceName.iter().position(|c| *c == 0).unwrap_or(32)],
        );
        let color = if format.dwEffects.0 & CFE_AUTOCOLOR.0 != 0 {
            0
        } else {
            format.crTextColor.0
        };
        html.push_str(&format!("<span style=\"font-family:'{}';font-size:{}pt;color:#{:02x}{:02x}{:02x};font-weight:{};font-style:{};text-decoration:{}\">{}</span>",
            escape(&face), format.yHeight as f64 / 20.0, color & 255, (color >> 8) & 255, (color >> 16) & 255,
            if format.dwEffects.0 & CFE_BOLD.0 != 0 { "bold" } else { "normal" },
            if format.dwEffects.0 & CFE_ITALIC.0 != 0 { "italic" } else { "normal" },
            if format.dwEffects.0 & CFE_UNDERLINE.0 != 0 { "underline" } else { "none" }, escape(&String::from_utf16_lossy(units))));
    }
}
pub unsafe fn html(body: HWND) -> String {
    unsafe {
        let saved = selection(body);
        let mut scroll = POINT::default();
        SendMessageW(body, EM_GETSCROLLPOS, 0, &mut scroll as *mut _ as isize);
        SendMessageW(body, WM_SETREDRAW, 0, 0);
        let mut units = vec![0u16; GetWindowTextLengthW(body) as usize + 1];
        let get = GETTEXTEX {
            cb: (units.len() * 2) as u32,
            codepage: 1200,
            ..Default::default()
        };
        let len = SendMessageW(
            body,
            EM_GETTEXTEX,
            &get as *const _ as usize,
            units.as_mut_ptr() as isize,
        )
        .max(0) as usize;
        units.truncate(len);
        let mut html = "<!doctype html><html><body>".to_owned();
        let mut offset = 0;
        for paragraph_text in units.split(|c| *c == 13) {
            select(
                body,
                CHARRANGE {
                    cpMin: offset as i32,
                    cpMax: offset as i32,
                },
            );
            let mut para = PARAFORMAT {
                cbSize: std::mem::size_of::<PARAFORMAT>() as u32,
                ..Default::default()
            };
            SendMessageW(body, EM_GETPARAFORMAT, 0, &mut para as *mut _ as isize);
            let bullet = para.wNumbering == PFN_BULLET;
            if bullet {
                html.push_str("<ul><li>");
            }
            html.push_str(&format!(
                "<p style=\"margin:0;white-space:pre-wrap;text-align:{}\">",
                match para.wAlignment {
                    PFA_CENTER => "center",
                    PFA_RIGHT => "right",
                    _ => "left",
                }
            ));
            spans(body, paragraph_text, offset, &mut html);
            if paragraph_text.is_empty() {
                html.push_str("<br>");
            }
            html.push_str("</p>");
            if bullet {
                html.push_str("</li></ul>");
            }
            offset += paragraph_text.len() + 1;
        }
        html.push_str("</body></html>");
        select(body, saved);
        SendMessageW(body, EM_SETSCROLLPOS, 0, &scroll as *const _ as isize);
        SendMessageW(body, WM_SETREDRAW, 1, 0);
        windows_sys::Win32::Graphics::Gdi::InvalidateRect(body, null(), 0);
        html
    }
}
