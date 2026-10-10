//! Native provider form controls and DPI layout.
use super::*;

impl OAuthSettings {
    pub(super) unsafe fn build(&mut self, window: HWND) {
        unsafe {
            self.layout
                .control(window, "STATIC", "&Provider:", 0, 0, [28, 32, 178, 26]);
            self.provider = self.layout.control(
                window,
                "COMBOBOX",
                "",
                WS_TABSTOP | CBS_DROPDOWNLIST as u32,
                PROVIDER,
                [220, 28, 240, 140],
            );
            for value in ["Microsoft", "Google", "Custom"] {
                SendMessageW(
                    self.provider,
                    CB_ADDSTRING,
                    0,
                    wide(value).as_ptr() as isize,
                );
            }
            for (index, id, label, y, height) in [
                (0, CLIENT_ID, "Client &ID:", 70, 26),
                (1, CLIENT_SECRET, "Client &secret (optional):", 110, 26),
                (
                    2,
                    AUTHORIZATION_ENDPOINT,
                    "&Authorization endpoint:",
                    176,
                    26,
                ),
                (3, TOKEN_ENDPOINT, "&Token endpoint:", 216, 26),
                (4, SCOPES, "Sc&opes (space separated):", 256, 52),
            ] {
                self.layout
                    .control(window, "STATIC", label, 0, 0, [28, y + 3, 178, 26]);
                let style = WS_TABSTOP
                    | if index == 1 {
                        ES_PASSWORD as u32 | ES_AUTOHSCROLL as u32
                    } else if index == 4 {
                        ES_MULTILINE as u32 | ES_AUTOVSCROLL as u32 | WS_VSCROLL
                    } else {
                        ES_AUTOHSCROLL as u32
                    };
                let edit =
                    self.layout
                        .control(window, "EDIT", "", style, id, [220, y, 580, height]);
                self.edits[index] = edit;
                SendMessageW(
                    edit,
                    EM_SETLIMITTEXT,
                    if index == 0 { 1024 } else { 8192 },
                    0,
                );
            }
            self.clear_secret = self.layout.control(
                window,
                "BUTTON",
                "Remove saved client secret",
                WS_TABSTOP | BS_AUTOCHECKBOX as u32,
                CLEAR_SECRET,
                [220, 140, 580, 24],
            );
            self.layout.control(
                window,
                "STATIC",
                "&Redirect host:",
                0,
                0,
                [28, 324, 178, 26],
            );
            self.redirect_host = self.layout.control(
                window,
                "COMBOBOX",
                "",
                WS_TABSTOP | CBS_DROPDOWNLIST as u32,
                REDIRECT_HOST,
                [220, 320, 240, 130],
            );
            for value in ["127.0.0.1 (dynamic port)", "localhost (dynamic port)"] {
                SendMessageW(
                    self.redirect_host,
                    CB_ADDSTRING,
                    0,
                    wide(value).as_ptr() as isize,
                );
            }
            self.hint = self
                .layout
                .control(window, "STATIC", "", 0, 0, [28, 358, 772, 42]);
            self.status = self
                .layout
                .control(window, "STATIC", "", 0, 0, [28, 410, 772, 46]);
            self.sign_in = self.layout.control(
                window,
                "BUTTON",
                "&Sign in",
                WS_TABSTOP,
                SIGN_IN,
                [220, 466, 120, 30],
            );
            self.sign_out = self.layout.control(
                window,
                "BUTTON",
                "Sign &out",
                WS_TABSTOP,
                SIGN_OUT,
                [352, 466, 120, 30],
            );
            self.save = self.layout.control(
                window,
                "BUTTON",
                "&OK",
                WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
                IDOK as u16,
                [584, 530, 96, 30],
            );
            self.layout.control(
                window,
                "BUTTON",
                "&Cancel",
                WS_TABSTOP,
                IDCANCEL as u16,
                [692, 530, 108, 30],
            );
            self.fill();
            self.status(false, if self.account.signed_in() { "Signed in. OK keeps these settings in Account Properties." } else { "Signed out. Sign in opens your default browser. A registered desktop client ID is required." });
            self.layout.update(window, dpi::window_dpi(window));
        }
    }
    pub(super) unsafe fn fill(&self) {
        unsafe {
            let config = &self.account.config;
            SendMessageW(
                self.provider,
                CB_SETCURSEL,
                match config.provider {
                    OAuthProvider::Microsoft => 0,
                    OAuthProvider::Google => 1,
                    OAuthProvider::Custom => 2,
                },
                0,
            );
            for (edit, value) in [
                (self.edits[0], config.client_id.as_str()),
                (self.edits[1], ""),
                (self.edits[2], config.authorization_endpoint.as_str()),
                (self.edits[3], config.token_endpoint.as_str()),
                (self.edits[4], config.scopes.join(" ").as_str()),
            ] {
                set_window_text(edit, value);
            }
            SendMessageW(self.clear_secret, BM_SETCHECK, BST_UNCHECKED as usize, 0);
            SendMessageW(
                self.redirect_host,
                CB_SETCURSEL,
                usize::from(config.loopback_host == LoopbackHost::Localhost),
                0,
            );
            self.enable_fields();
        }
    }
    pub(super) unsafe fn input(&self) -> ConfigInput {
        unsafe {
            ConfigInput {
                provider: match SendMessageW(self.provider, CB_GETCURSEL, 0, 0) {
                    1 => OAuthProvider::Google,
                    2 => OAuthProvider::Custom,
                    _ => OAuthProvider::Microsoft,
                },
                client_id: text(self.edits[0]),
                client_secret: text(self.edits[1]),
                clear_secret: SendMessageW(self.clear_secret, BM_GETCHECK, 0, 0)
                    == BST_CHECKED as isize,
                authorization_endpoint: text(self.edits[2]),
                token_endpoint: text(self.edits[3]),
                scopes: text(self.edits[4]),
                loopback_host: if SendMessageW(self.redirect_host, CB_GETCURSEL, 0, 0) == 1 {
                    LoopbackHost::Localhost
                } else {
                    LoopbackHost::IpAddress
                },
            }
        }
    }
    pub(super) unsafe fn status(&mut self, error: bool, message: &str) {
        unsafe {
            self.error = error;
            set_window_text(self.status, message);
            InvalidateRect(self.status, null(), 1);
        }
    }
    pub(super) unsafe fn enable_fields(&self) {
        unsafe {
            let ready = self.worker.is_none();
            let custom = self.account.config.provider == OAuthProvider::Custom;
            let secret = self.account.config.provider != OAuthProvider::Microsoft;
            for window in [self.provider, self.edits[0], self.sign_in, self.save] {
                EnableWindow(window, i32::from(ready));
            }
            EnableWindow(self.sign_out, i32::from(ready && self.account.signed_in()));
            for window in [self.edits[1], self.clear_secret] {
                EnableWindow(window, i32::from(ready && secret));
            }
            for window in [
                self.edits[2],
                self.edits[3],
                self.edits[4],
                self.redirect_host,
            ] {
                EnableWindow(window, i32::from(ready && custom));
            }
            set_window_text(
                self.hint,
                match self.account.config.provider {
                    OAuthProvider::Microsoft => {
                        "Register a public desktop client with http://localhost as the redirect. The callback uses a dynamic local port; no client secret is used."
                    }
                    OAuthProvider::Google => {
                        "Use a Google Desktop app client ID. Redirect: http://127.0.0.1:<dynamic port>/. Leave the secret empty to keep a saved credential for this client ID."
                    }
                    OAuthProvider::Custom => {
                        "Use HTTPS authorization and token endpoints supporting PKCE (S256). Register the chosen loopback redirect with a dynamic local port."
                    }
                },
            );
        }
    }
    pub(super) unsafe fn change_provider(&mut self) {
        unsafe {
            if self.worker.is_some() {
                return;
            }
            let provider = self.input().provider;
            if provider == self.account.config.provider {
                return;
            }
            self.account = OAuthAccount {
                config: preset(provider, String::new()),
                tokens: None,
            };
            self.fill();
            self.status(
                false,
                "Enter the registered client ID for this provider, then choose Sign in.",
            );
        }
    }
}
