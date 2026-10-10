//! Decode borrowed Win32 message payloads before dispatching owned UI events.
//!
//! Windows can synchronously reenter a window procedure during a control call.
//! The host queues events while its window is borrowed; it never creates a
//! second mutable reference to the window state.
use super::{
    DISPATCH, HTML_STATUS, READER_DISPATCH,
    account_window::{ACCOUNT_SUBMITTED, AccountSubmission},
    command_ids,
    main_window::MainWindow,
};
use crate::store::Store;
use crate::{
    app::{Command, SortColumn},
    platform::{classic, dpi},
};
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
    sync::mpsc::{self, Receiver},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::ClientToScreen,
    System::DataExchange::COPYDATASTRUCT,
    UI::{Controls::*, WindowsAndMessaging::*},
};

const PENDING: u32 = WM_APP + 2;

pub(super) enum WindowEvent {
    Create(HWND),
    Resize {
        width: i32,
        height: i32,
    },
    Command {
        command: Command,
        composer: Option<HWND>,
    },
    Refresh,
    FolderSelected(usize),
    MessageSelected,
    MessageActivated(usize),
    ReaderCommand {
        command: Command,
        reader: HWND,
    },
    HtmlStatusChanged,
    ContextMenu {
        control: HWND,
        point: POINT,
    },
    Sort(SortColumn),
    DpiChanged {
        value: u32,
        suggested: RECT,
    },
    BeginDrag {
        x: i32,
        y: i32,
    },
    Drag {
        x: i32,
        y: i32,
    },
    EndDrag,
    Launch(String),
    ProcessLaunches,
    Poll,
    Close,
    Destroy,
}

pub(super) struct WindowHost {
    state: RefCell<MainWindow>,
    queued: RefCell<VecDeque<WindowEvent>>,
    account_rx: Receiver<AccountSubmission>,
    destroyed: Cell<bool>,
    suppress_notifications: Rc<Cell<bool>>,
}

impl WindowHost {
    pub(super) fn new(store: Store) -> Self {
        let (account_tx, account_rx) = mpsc::channel();
        let suppress_notifications = Rc::new(Cell::new(true));
        Self {
            state: RefCell::new(MainWindow::new(
                store,
                account_tx,
                suppress_notifications.clone(),
            )),
            queued: RefCell::new(VecDeque::new()),
            account_rx,
            destroyed: Cell::new(false),
            suppress_notifications,
        }
    }

    pub(super) fn with_window<T>(&self, action: impl FnOnce(&mut MainWindow) -> T) -> T {
        let result = action(&mut self.state.borrow_mut());
        self.drain();
        result
    }

    fn dispatch(&self, event: WindowEvent) {
        if matches!(event, WindowEvent::Destroy) {
            self.destroyed.set(true);
            self.queued.borrow_mut().clear();
        } else if self.destroyed.get() {
            return;
        }
        self.queued.borrow_mut().push_back(event);
        self.drain();
    }

    fn drain(&self) {
        let Ok(mut state) = self.state.try_borrow_mut() else {
            return;
        };
        loop {
            // Release the queue borrow before calling any Windows API.
            let event = self.queued.borrow_mut().pop_front();
            let Some(event) = event else {
                break;
            };
            state.on_event(event);
        }
    }

    fn submit_accounts(&self) {
        while let Ok(submission) = self.account_rx.try_recv() {
            // A synchronous save must either finish or reject before returning.
            // Never enqueue a transaction the form would report as failed.
            let result = if self.destroyed.get() {
                Err("Mail window closed".into())
            } else if let Ok(mut state) = self.state.try_borrow_mut() {
                state.apply_account_change(submission.change)
            } else {
                Err("Mail window is busy; try saving the account again".into())
            };
            let _ = submission.reply.send(result);
        }
        self.drain();
    }
}

/// # Safety
/// Called by Windows with message-specific valid, borrowed payloads. The host
/// pointer is installed at WM_NCCREATE and stays boxed through the message loop.
pub(super) unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wp: usize,
    lp: isize,
) -> isize {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lp as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
            return 1;
        }
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WindowHost;
        if pointer.is_null() {
            return DefWindowProcW(hwnd, msg, wp, lp);
        }
        // SAFETY: run() owns the boxed host until all window callbacks finish.
        let host = &*pointer;
        let event = match msg {
            WM_CREATE => WindowEvent::Create(hwnd),
            WM_SIZE => WindowEvent::Resize {
                width: (lp as u32 & 0xffff) as i32,
                height: ((lp as u32 >> 16) & 0xffff) as i32,
            },
            READER_DISPATCH => {
                let Some(command) = command_ids::decode((wp & 0xffff) as u16) else {
                    return 0;
                };
                WindowEvent::ReaderCommand {
                    command,
                    reader: lp as HWND,
                }
            }
            WM_COMMAND | DISPATCH => {
                let id = (wp & 0xffff) as u16;
                let notification = (wp >> 16) as u16;
                if (id == command_ids::SEARCH && notification == EN_CHANGE as u16)
                    || (id == command_ids::FILTER && notification == CBN_SELCHANGE as u16)
                {
                    WindowEvent::Refresh
                } else if let Some(command) = command_ids::decode(id) {
                    WindowEvent::Command {
                        command,
                        composer: (msg == DISPATCH && lp != 0).then_some(lp as HWND),
                    }
                } else {
                    return 0;
                }
            }
            WM_NOTIFY => {
                if let Some(result) = classic::custom_draw(lp) {
                    return result;
                }
                if host.suppress_notifications.get() {
                    return 0;
                }
                // NMLVKEYDOWN is packed; WM_NOTIFY does not promise NMHDR alignment.
                let header = std::ptr::read_unaligned(lp as *const NMHDR);
                if header.idFrom == command_ids::FOLDER as usize && header.code == TVN_SELCHANGEDW {
                    let change = &*(lp as *const NMTREEVIEWW);
                    WindowEvent::FolderSelected(change.itemNew.lParam as usize)
                } else if header.idFrom == command_ids::LIST as usize
                    && header.code == LVN_ITEMCHANGED
                {
                    let change = &*(lp as *const NMLISTVIEW);
                    if change.uNewState & LVIS_SELECTED == 0
                        || change.uOldState & LVIS_SELECTED != 0
                    {
                        return 0;
                    }
                    WindowEvent::MessageSelected
                } else if header.idFrom == command_ids::LIST as usize
                    && matches!(header.code, NM_DBLCLK | NM_RCLICK)
                {
                    let activation = std::ptr::read_unaligned(lp as *const NMITEMACTIVATE);
                    // iItem can be -1 for clicks outside the icon/first label.
                    // Subject, sender and date columns still activate their row.
                    let index = usize::try_from(activation.iItem).ok().or_else(|| {
                        let mut hit = LVHITTESTINFO {
                            pt: activation.ptAction,
                            ..Default::default()
                        };
                        usize::try_from(SendMessageW(
                            header.hwndFrom,
                            LVM_SUBITEMHITTEST,
                            0,
                            &mut hit as *mut _ as isize,
                        ))
                        .ok()
                    });
                    let Some(index) = index else {
                        return isize::from(header.code == NM_RCLICK);
                    };
                    if header.code == NM_DBLCLK {
                        WindowEvent::MessageActivated(index)
                    } else {
                        let mut point = activation.ptAction;
                        ClientToScreen(header.hwndFrom, &mut point);
                        host.dispatch(WindowEvent::ContextMenu {
                            control: header.hwndFrom,
                            point,
                        });
                        // Suppress the list's default WM_CONTEXTMENU duplicate.
                        return 1;
                    }
                } else if header.idFrom == command_ids::LIST as usize && header.code == LVN_KEYDOWN
                {
                    let key = &*(lp as *const NMLVKEYDOWN);
                    if key.wVKey == windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_RETURN {
                        host.dispatch(WindowEvent::Command {
                            command: Command::OpenMessage,
                            composer: None,
                        });
                        return 0;
                    }
                    if key.wVKey != windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_DELETE {
                        return 0;
                    }
                    let permanent = windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyState(
                        windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_SHIFT as i32,
                    ) < 0;
                    WindowEvent::Command {
                        command: if permanent {
                            Command::DeletePermanently
                        } else {
                            Command::Delete
                        },
                        composer: None,
                    }
                } else if header.idFrom == command_ids::LIST as usize
                    && header.code == LVN_COLUMNCLICK
                {
                    let change = &*(lp as *const NMLISTVIEW);
                    let Some(column) = command_ids::sort_column(change.iSubItem) else {
                        return 0;
                    };
                    WindowEvent::Sort(column)
                } else {
                    return 0;
                }
            }
            WM_DPICHANGED => WindowEvent::DpiChanged {
                value: (wp & 0xffff) as u32,
                suggested: *(lp as *const RECT),
            },
            WM_CONTEXTMENU => WindowEvent::ContextMenu {
                control: wp as HWND,
                point: POINT {
                    x: lp as i16 as i32,
                    y: (lp >> 16) as i16 as i32,
                },
            },
            WM_GETMINMAXINFO => {
                // Native queries need an immediate answer, including during reentry.
                let value = dpi::window_dpi(hwnd);
                let info = &mut *(lp as *mut MINMAXINFO);
                info.ptMinTrackSize = POINT {
                    x: dpi::scale(650, value),
                    y: dpi::scale(420, value),
                };
                return 0;
            }
            WM_LBUTTONDOWN => WindowEvent::BeginDrag {
                x: lp as i16 as i32,
                y: (lp >> 16) as i16 as i32,
            },
            WM_MOUSEMOVE => WindowEvent::Drag {
                x: lp as i16 as i32,
                y: (lp >> 16) as i16 as i32,
            },
            WM_LBUTTONUP => WindowEvent::EndDrag,
            WM_COPYDATA => {
                let packet = &*(lp as *const COPYDATASTRUCT);
                if packet.dwData != 1
                    || packet.lpData.is_null()
                    || packet.cbData > 65536
                    || !packet.cbData.is_multiple_of(2)
                {
                    return 0;
                }
                let words = std::slice::from_raw_parts(
                    packet.lpData.cast::<u16>(),
                    packet.cbData as usize / 2,
                );
                let end = words.iter().position(|&c| c == 0).unwrap_or(words.len());
                // Copy the caller-owned buffer before this synchronous message returns.
                host.dispatch(WindowEvent::Launch(String::from_utf16_lossy(&words[..end])));
                PostMessageW(hwnd, PENDING, 0, 0);
                return 1;
            }
            PENDING => WindowEvent::ProcessLaunches,
            ACCOUNT_SUBMITTED => {
                host.submit_accounts();
                return 0;
            }
            HTML_STATUS => WindowEvent::HtmlStatusChanged,
            WM_TIMER => WindowEvent::Poll,
            WM_CLOSE => WindowEvent::Close,
            WM_DESTROY => WindowEvent::Destroy,
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            _ => return DefWindowProcW(hwnd, msg, wp, lp),
        };
        host.dispatch(event);
        0
    }
}
