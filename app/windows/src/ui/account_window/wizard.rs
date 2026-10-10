//! Provider selection and identity steps; properties provides final review/sign-in.
use super::*;
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::Graphics::Gdi::*;
const NEXT: u16 = 760;
const BACK: u16 = 761;
const PROVIDER: u16 = 762;
const EMAIL: u16 = 763;
const NAME: u16 = 764;

pub(super) struct Provider {
    name: &'static str,
    imap: &'static str,
    smtp: &'static str,
    smtp_port: u16,
    pub(super) hint: &'static str,
    oauth: bool,
}
// Reviewed against provider documentation; see providers.md.
const PROVIDERS: &[Provider] = &[
    Provider {
        name: "iCloud Mail",
        imap: "imap.mail.me.com",
        smtp: "smtp.mail.me.com",
        smtp_port: 587,
        hint: "Use an Apple app-specific password, not your Apple Account password.",
        oauth: false,
    },
    Provider {
        name: "Gmail / Google Workspace",
        imap: "imap.gmail.com",
        smtp: "smtp.gmail.com",
        smtp_port: 587,
        hint: "Use a Google app password (requires 2-Step Verification), or configure OAuth in the next window. Your administrator may restrict access.",
        oauth: false,
    },
    Provider {
        name: "Outlook.com / Hotmail",
        imap: "outlook.office365.com",
        smtp: "smtp-mail.outlook.com",
        smtp_port: 587,
        hint: "Enable IMAP in Outlook.com settings. OAuth 2.0 is required: configure your Microsoft client ID and sign in in the next window.",
        oauth: true,
    },
    Provider {
        name: "Yahoo Mail",
        imap: "imap.mail.yahoo.com",
        smtp: "smtp.mail.yahoo.com",
        smtp_port: 465,
        hint: "Generate a Yahoo app password for this email application.",
        oauth: false,
    },
    Provider {
        name: "QQ Mail",
        imap: "imap.qq.com",
        smtp: "smtp.qq.com",
        smtp_port: 465,
        hint: "Enable IMAP/SMTP in QQ Mail settings and use the authorization code as your password.",
        oauth: false,
    },
    Provider {
        name: "Other / manual setup",
        imap: "",
        smtp: "",
        smtp_port: 587,
        hint: "Enter your provider's server addresses and login method in the next window.",
        oauth: false,
    },
];

struct Wizard {
    owner: HWND,
    layout: WindowLayout,
    pages: [Vec<HWND>; 2],
    page: usize,
    provider: usize,
    email: HWND,
    name: HWND,
    hint: HWND,
    heading: HWND,
    steps: HWND,
}
impl Wizard {
    unsafe fn item(
        &mut self,
        window: HWND,
        page: usize,
        class: &str,
        label: &str,
        style: u32,
        id: u16,
        bounds: [i32; 4],
    ) -> HWND {
        let control = unsafe { self.layout.control(window, class, label, style, id, bounds) };
        self.pages[page].push(control);
        control
    }
    unsafe fn build(&mut self, window: HWND) {
        unsafe {
            self.heading = self
                .layout
                .control(window, "STATIC", "", 0, 0, [76, 22, 436, 28]);
            self.item(
                window,
                0,
                "STATIC",
                "Choose your email service. We will fill in the server and encryption settings.",
                0,
                0,
                [24, 108, 488, 36],
            );
            let list = self.item(
                window,
                0,
                "LISTBOX",
                "",
                WS_TABSTOP | LBS_NOTIFY as u32 | WS_BORDER,
                PROVIDER,
                [24, 150, 488, 132],
            );
            for provider in PROVIDERS {
                SendMessageW(list, LB_ADDSTRING, 0, wide(provider.name).as_ptr() as isize);
            }
            SendMessageW(list, LB_SETCURSEL, 0, 0);
            self.item(
                window,
                1,
                "STATIC",
                "Email address",
                0,
                0,
                [24, 110, 488, 20],
            );
            self.email = self.item(
                window,
                1,
                "EDIT",
                "",
                WS_TABSTOP | ES_AUTOHSCROLL as u32,
                EMAIL,
                [24, 138, 488, 26],
            );
            self.item(
                window,
                1,
                "STATIC",
                "Account name (optional)",
                0,
                0,
                [24, 184, 488, 20],
            );
            self.name = self.item(
                window,
                1,
                "EDIT",
                "",
                WS_TABSTOP | ES_AUTOHSCROLL as u32,
                NAME,
                [24, 212, 488, 26],
            );
            for control in [self.email, self.name] {
                SendMessageW(control, EM_SETLIMITTEXT, 320, 0);
            }
            self.hint = self.item(window, 1, "STATIC", "", 0, 0, [24, 258, 488, 54]);
            self.layout.control(
                window,
                "BUTTON",
                "‹ Back",
                WS_TABSTOP,
                BACK,
                [212, 330, 90, 30],
            );
            self.layout.control(
                window,
                "BUTTON",
                "Next ›",
                WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
                NEXT,
                [314, 330, 96, 30],
            );
            self.layout.control(
                window,
                "BUTTON",
                "Cancel",
                WS_TABSTOP,
                IDCANCEL as u16,
                [422, 330, 90, 30],
            );
            self.layout.update(window, dpi::window_dpi(window));
            self.layout
                .metrics
                .apply_font(self.heading, dpi::FontWeight::Bold);
            self.steps = self
                .layout
                .control(window, "STATIC", "", 0, 0, [76, 56, 436, 20]);
            self.layout.update(window, dpi::window_dpi(window));
            self.select(window, 0);
            self.layout
                .metrics
                .apply_font(self.heading, dpi::FontWeight::Bold);
        }
    }
    unsafe fn select(&mut self, window: HWND, page: usize) {
        unsafe {
            self.page = page;
            for (index, controls) in self.pages.iter().enumerate() {
                for control in controls {
                    ShowWindow(*control, if index == page { SW_SHOW } else { SW_HIDE });
                }
            }
            set_window_text(
                self.heading,
                if page == 0 {
                    "Choose your email service"
                } else {
                    "Enter your email address"
                },
            );
            set_window_text(
                self.steps,
                if page == 0 {
                    "Step 1 of 3 · Service → Account → Review"
                } else {
                    "Step 2 of 3 · Service → Account → Review"
                },
            );
            set_window_text(self.hint, PROVIDERS[self.provider].hint);
            InvalidateRect(window, null(), 1);
            set_window_text(
                GetDlgItem(window, NEXT as i32),
                if page == 0 { "Next ›" } else { "Review ›" },
            );
            EnableWindow(GetDlgItem(window, BACK as i32), i32::from(page > 0));
            SetFocus(if page == 0 {
                GetDlgItem(window, PROVIDER as i32)
            } else {
                self.email
            });
        }
    }
}

pub(super) unsafe fn open(manager: HWND) {
    unsafe {
        let host = GetWindowLongPtrW(manager, GWLP_USERDATA) as *const manager::ManagerHost;
        if host.is_null() {
            return;
        }
        let existing = (*host).property.get();
        if IsWindow(existing) != 0 {
            SetForegroundWindow(existing);
            return;
        }
        let class = wide("WinMailRustAccountWizard");
        if let Err(error) = register_class(&class, Some(procedure)) {
            manager::error(manager, &error);
            return;
        }
        let mut state = Box::new(Wizard {
            owner: manager,
            layout: WindowLayout::default(),
            pages: std::array::from_fn(|_| Vec::new()),
            page: 0,
            provider: 0,
            email: null_mut(),
            name: null_mut(),
            hint: null_mut(),
            heading: null_mut(),
            steps: null_mut(),
        });
        let value = dpi::window_dpi(manager);
        let window = CreateWindowExW(
            WS_EX_CONTROLPARENT | WS_EX_DLGMODALFRAME,
            class.as_ptr(),
            wide("New Account Wizard — Windows Mail Vista").as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            dpi::scale(554, value),
            dpi::scale(414, value),
            manager,
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        if window.is_null() {
            manager::error(manager, "Could not open the account wizard.");
            return;
        }
        SetWindowLongPtrW(window, GWLP_USERDATA, (&mut *state as *mut Wizard) as isize);
        state.build(window);
        let _ = Box::into_raw(state);
        (*host).property.set(window);
        EnableWindow(manager, 0);
        ShowWindow(window, SW_SHOW);
        SetForegroundWindow(window);
    }
}
unsafe extern "system" fn procedure(window: HWND, message: u32, wp: usize, lp: isize) -> isize {
    unsafe {
        let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Wizard;
        if pointer.is_null() {
            return DefWindowProcW(window, message, wp, lp);
        }
        match message {
            WM_COMMAND => {
                match (wp & 0xffff) as u16 {
                    NEXT => {
                        if (*pointer).page == 0 {
                            let selected = SendMessageW(
                                GetDlgItem(window, PROVIDER as i32),
                                LB_GETCURSEL,
                                0,
                                0,
                            );
                            if selected >= 0 && (selected as usize) < PROVIDERS.len() {
                                (*pointer).provider = selected as usize;
                                (*pointer).select(window, 1);
                            }
                        } else {
                            let email = text((*pointer).email).trim().to_owned();
                            if !matches!(crate::message::addresses(&email),Ok(addresses) if addresses.len()==1)
                            {
                                MessageBoxW(
                                    window,
                                    wide("Enter one valid email address.").as_ptr(),
                                    wide("Email address").as_ptr(),
                                    MB_ICONINFORMATION,
                                );
                                SetFocus((*pointer).email);
                                return 0;
                            }
                            let name = text((*pointer).name).trim().to_owned();
                            let provider = &PROVIDERS[(*pointer).provider];
                            let manager = (*pointer).owner;
                            DestroyWindow(window);
                            properties::open_preset(manager, provider, &email, &name);
                        }
                    }
                    BACK => (*pointer).select(window, 0),
                    id if id == IDCANCEL as u16 => {
                        DestroyWindow(window);
                    }
                    _ => {}
                }
                0
            }
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let dc = BeginPaint(window, &mut paint);
                let mut rect = RECT::default();
                GetClientRect(window, &mut rect);
                let value = dpi::window_dpi(window);
                let header = dpi::scale(92, value);
                for y in 0..header {
                    let t = y as f32 / header as f32;
                    let r = (229.0 + 18.0 * t) as u32;
                    let g = (241.0 + 10.0 * t) as u32;
                    let brush = CreateSolidBrush(r | (g << 8) | (255 << 16));
                    FillRect(
                        dc,
                        &RECT {
                            left: 0,
                            top: y,
                            right: rect.right,
                            bottom: y + 1,
                        },
                        brush,
                    );
                    DeleteObject(brush);
                }
                let footer = dpi::scale(318, value);
                FillRect(
                    dc,
                    &RECT {
                        left: 0,
                        top: footer,
                        right: rect.right,
                        bottom: rect.bottom,
                    },
                    GetSysColorBrush(COLOR_BTNFACE),
                );
                let line = CreateSolidBrush(crate::platform::theme::panel(0x00ddd6cf));
                for y in [header - 1, footer] {
                    FillRect(
                        dc,
                        &RECT {
                            left: 0,
                            top: y,
                            right: rect.right,
                            bottom: y + 1,
                        },
                        line,
                    );
                }
                DeleteObject(line);
                let icon = SendMessageW(window, WM_GETICON, ICON_BIG as usize, 0) as HICON;
                DrawIconEx(
                    dc,
                    dpi::scale(24, value),
                    dpi::scale(25, value),
                    icon,
                    dpi::scale(32, value),
                    dpi::scale(32, value),
                    0,
                    null_mut(),
                    DI_NORMAL,
                );
                EndPaint(window, &paint);
                0
            }
            WM_CTLCOLORSTATIC => {
                SetBkMode(wp as HDC, TRANSPARENT as i32);
                let header = lp as HWND == (*pointer).heading || lp as HWND == (*pointer).steps;
                SetTextColor(
                    wp as HDC,
                    if header {
                        0x00993300
                    } else {
                        GetSysColor(COLOR_WINDOWTEXT)
                    },
                );
                if header {
                    GetStockObject(HOLLOW_BRUSH) as isize
                } else {
                    GetSysColorBrush(COLOR_WINDOW) as isize
                }
            }
            WM_DPICHANGED => {
                (*pointer).layout.update(window, (wp & 0xffff) as u32);
                (*pointer)
                    .layout
                    .metrics
                    .apply_font((*pointer).heading, dpi::FontWeight::Bold);
                dpi::suggested(window, lp);
                0
            }
            WM_CLOSE => {
                DestroyWindow(window);
                0
            }
            WM_NCDESTROY => {
                let owner = (*pointer).owner;
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                drop(Box::from_raw(pointer));
                if IsWindow(owner) != 0 {
                    SendMessageW(owner, PROPERTY_CLOSED, window as usize, 0);
                    EnableWindow(owner, 1);
                    SetForegroundWindow(owner);
                }
                DefWindowProcW(window, message, wp, lp)
            }
            _ => DefWindowProcW(window, message, wp, lp),
        }
    }
}

pub(super) unsafe fn populate(window: HWND, provider: &Provider, email: &str, name: &str) {
    unsafe {
        let host = GetWindowLongPtrW(window, GWLP_USERDATA) as *const properties::PropertyHost;
        if host.is_null() {
            return;
        }
        let fields = [
            (Field::Name, name),
            (Field::Email, email),
            (Field::Username, email),
            (Field::IncomingHost, provider.imap),
            (Field::SmtpHost, provider.smtp),
        ];
        for (field, value) in fields {
            set_window_text(GetDlgItem(window, 300 + field as i32), value);
        }
        set_window_text(GetDlgItem(window, 300 + Field::IncomingPort as i32), "993");
        set_window_text(
            GetDlgItem(window, 300 + Field::SmtpPort as i32),
            &provider.smtp_port.to_string(),
        );
        SendMessageW(GetDlgItem(window, 320), CB_SETCURSEL, 0, 0);
        SendMessageW(GetDlgItem(window, 321), CB_SETCURSEL, 0, 0);
        SendMessageW(
            GetDlgItem(window, 322),
            CB_SETCURSEL,
            usize::from(provider.smtp_port == 587),
            0,
        );
        if provider.oauth {
            SendMessageW(
                GetDlgItem(window, properties::AUTHENTICATION as i32),
                CB_SETCURSEL,
                1,
                0,
            );
            SendMessageW(
                window,
                WM_COMMAND,
                properties::AUTHENTICATION as usize | ((CBN_SELCHANGE as usize) << 16),
                GetDlgItem(window, properties::AUTHENTICATION as i32) as isize,
            );
        }
        set_window_text(window, "3 of 3 — Review & sign in");
    }
}
