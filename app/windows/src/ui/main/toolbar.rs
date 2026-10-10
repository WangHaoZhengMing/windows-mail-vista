//! Reserve space for search/filter, collapsing captions when the window narrows.
use crate::app::Command;
use crate::ui::shared::commands::{self, command_id};
use windows_sys::Win32::{
    Foundation::{HWND, RECT},
    UI::{Controls::*, WindowsAndMessaging::SendMessageW},
};

pub(super) unsafe fn fit(toolbar: HWND, available_width: i32) {
    unsafe {
        let extended = SendMessageW(toolbar, TB_GETEXTENDEDSTYLE, 0, 0);
        SendMessageW(
            toolbar,
            TB_SETEXTENDEDSTYLE,
            0,
            extended | TBSTYLE_EX_MIXEDBUTTONS as isize,
        );
        let count = SendMessageW(toolbar, TB_BUTTONCOUNT, 0, 0);
        if count <= 0 {
            return;
        }
        for presentation in [
            Presentation::Full,
            Presentation::MailActions,
            Presentation::Primary,
            Presentation::Icons,
        ] {
            captions(toolbar, presentation);
            SendMessageW(toolbar, TB_AUTOSIZE, 0, 0);
            let mut last = RECT::default();
            SendMessageW(
                toolbar,
                TB_GETITEMRECT,
                (count - 1) as usize,
                &mut last as *mut _ as isize,
            );
            if last.right <= available_width {
                break;
            }
        }
    }
}
#[derive(Clone, Copy)]
enum Presentation {
    Full,
    MailActions,
    Primary,
    Icons,
}
unsafe fn captions(toolbar: HWND, presentation: Presentation) {
    unsafe {
        let count = SendMessageW(toolbar, TB_BUTTONCOUNT, 0, 0);
        for index in 0..count {
            let mut button = TBBUTTON::default();
            if SendMessageW(
                toolbar,
                TB_GETBUTTON,
                index as usize,
                &mut button as *mut _ as isize,
            ) == 0
            {
                continue;
            }
            let show = match presentation {
                Presentation::Full => true,
                Presentation::MailActions => ![
                    i32::from(commands::PRINT),
                    i32::from(commands::CONTACTS),
                    i32::from(command_id(Command::Find)),
                ]
                .contains(&button.idCommand),
                Presentation::Primary => {
                    button.idCommand == i32::from(command_id(Command::NewMessage))
                }
                Presentation::Icons => false,
            };
            let style = (button.fsStyle & !(BTNS_SHOWTEXT as u8))
                | if show { BTNS_SHOWTEXT as u8 } else { 0 };
            if style != button.fsStyle {
                let info = TBBUTTONINFOW {
                    cbSize: std::mem::size_of::<TBBUTTONINFOW>() as u32,
                    dwMask: TBIF_BYINDEX | TBIF_STYLE,
                    fsStyle: style,
                    ..Default::default()
                };
                SendMessageW(
                    toolbar,
                    TB_SETBUTTONINFOW,
                    index as usize,
                    &info as *const _ as isize,
                );
            }
        }
    }
}
