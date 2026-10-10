use super::*;
use mail_core::oauth::OAuthTokens;
use windows_sys::Win32::{Foundation::RECT, UI::Input::KeyboardAndMouse::IsWindowEnabled};

fn signed_google() -> OAuthAccount {
    let mut config = OAuthConfig::google("registered-client-id");
    config.set_client_secret("saved-client-secret").unwrap();
    OAuthAccount {
        config,
        tokens: Some(OAuthTokens {
            protected: crate::account::protect(b"encrypted-token-fixture").unwrap(),
            expires_at: u64::MAX,
        }),
    }
}
fn input(account: &OAuthAccount) -> ConfigInput {
    ConfigInput {
        provider: account.config.provider,
        client_id: account.config.client_id.clone(),
        client_secret: String::new(),
        clear_secret: false,
        authorization_endpoint: account.config.authorization_endpoint.clone(),
        token_endpoint: account.config.token_endpoint.clone(),
        scopes: account.config.scopes.join(" "),
        loopback_host: account.config.loopback_host,
    }
}
#[test]
fn unchanged_oauth_configuration_preserves_encrypted_tokens_and_hidden_client_secret() {
    let original = signed_google();
    let fields = input(&original);
    let account = fields.account(&original).unwrap();
    assert_eq!(
        account.tokens.unwrap().protected,
        original.tokens.unwrap().protected
    );
    assert_eq!(
        account.config.client_secret_dpapi,
        original.config.client_secret_dpapi
    );
    assert_ne!(account.config.client_secret_dpapi, "saved-client-secret");
}
#[test]
fn changed_client_or_removed_secret_requires_a_new_authorization() {
    let original = signed_google();
    let mut fields = input(&original);
    fields.client_id = "different-client".into();
    let changed = fields.account(&original).unwrap();
    assert!(changed.tokens.is_none());
    assert!(
        !changed.config.has_client_secret(),
        "A new application must never inherit another application's secret"
    );
    fields.client_id = original.config.client_id.clone();
    fields.clear_secret = true;
    let account = fields.account(&original).unwrap();
    assert!(!account.config.has_client_secret());
    assert!(account.tokens.is_none());
    assert!(original.signed_in());
    assert!(original.config.has_client_secret());
}
#[test]
fn custom_endpoints_and_scopes_are_validated_before_any_worker_or_save() {
    let original = signed_google();
    let mut fields = input(&original);
    fields.provider = OAuthProvider::Custom;
    fields.client_id = "custom-client".into();
    fields.authorization_endpoint = "http://provider.example/authorize".into();
    assert!(fields.account(&original).is_err());
    fields.authorization_endpoint = "https://provider.example/authorize".into();
    fields.token_endpoint = "https://provider.example/token".into();
    fields.scopes = "offline_access\r\nmail.read mail.send".into();
    let account = fields.account(&original).unwrap();
    assert_eq!(
        account.config.scopes,
        ["offline_access", "mail.read", "mail.send"]
    );
    assert!(account.tokens.is_none());
}
struct NativeWindows {
    owner: HWND,
    oauth: HWND,
}
impl Drop for NativeWindows {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(self.oauth) != 0 {
                DestroyWindow(self.oauth);
            }
            if IsWindow(self.owner) != 0 {
                DestroyWindow(self.owner);
            }
        }
    }
}
unsafe fn show_fixture(previous: Option<OAuthAccount>) -> (NativeWindows, Receiver<OAuthAccount>) {
    unsafe {
        let owner = CreateWindowExW(
            0,
            wide("STATIC").as_ptr(),
            wide("OAuth owner fixture").as_ptr(),
            WS_OVERLAPPEDWINDOW,
            0,
            0,
            100,
            100,
            null_mut(),
            null_mut(),
            GetModuleHandleW(null()),
            null(),
        );
        assert!(!owner.is_null());
        let (sender, receiver) = mpsc::channel();
        let oauth = open(owner, previous, "reader@example.test".into(), sender).unwrap();
        ShowWindow(oauth, SW_HIDE);
        (NativeWindows { owner, oauth }, receiver)
    }
}
#[test]
fn native_provider_defaults_validation_dpi_save_and_cancel_do_not_launch_a_browser() {
    let _lock = crate::ui::WINDOW_TEST_LOCK.lock().unwrap();
    let _context = dpi::ThreadContext::enter();
    unsafe {
        let (windows, updates) = show_fixture(None);
        let oauth = windows.oauth;
        assert_eq!(IsWindowEnabled(windows.owner), 0);
        let provider = GetDlgItem(oauth, PROVIDER as i32);
        let client = GetDlgItem(oauth, CLIENT_ID as i32);
        assert_eq!(text(client), ""); // No application's client ID is bundled.
        SendMessageW(oauth, WM_COMMAND, SIGN_IN as usize, 0); // Invalid configuration stops before browser launch.
        let host = GetWindowLongPtrW(oauth, GWLP_USERDATA) as *const OAuthHost;
        assert!((*host).state.borrow().error);
        assert!((*host).state.borrow().worker.is_none());
        SendMessageW(provider, CB_SETCURSEL, 1, 0);
        SendMessageW(
            oauth,
            WM_COMMAND,
            PROVIDER as usize | ((CBN_SELCHANGE as usize) << 16),
            0,
        );
        set_window_text(client, "google-desktop-client");
        assert_eq!(
            SendMessageW(GetDlgItem(oauth, REDIRECT_HOST as i32), CB_GETCURSEL, 0, 0),
            0
        );
        assert_eq!(
            text(GetDlgItem(oauth, AUTHORIZATION_ENDPOINT as i32)),
            OAuthConfig::google("").authorization_endpoint
        );
        assert_eq!(
            IsWindowEnabled(GetDlgItem(oauth, AUTHORIZATION_ENDPOINT as i32)),
            0
        );
        assert_ne!(IsWindowEnabled(GetDlgItem(oauth, CLIENT_SECRET as i32)), 0);
        for value in [96, 144, 192, 288, 384, 96] {
            let rect = RECT {
                left: 0,
                top: 0,
                right: dpi::scale(WIDTH, value),
                bottom: dpi::scale(HEIGHT, value),
            };
            SendMessageW(
                oauth,
                WM_DPICHANGED,
                value as usize,
                &rect as *const _ as isize,
            );
            let mut bounds: RECT = std::mem::zeroed();
            GetWindowRect(client, &mut bounds);
            assert_eq!(bounds.right - bounds.left, dpi::scale(580, value));
            assert_eq!(text(client), "google-desktop-client");
            assert_ne!(SendMessageW(client, WM_GETFONT, 0, 0), 0);
        }
        SendMessageW(oauth, WM_COMMAND, IDOK as usize, 0);
        assert_eq!(IsWindow(oauth), 0);
        assert_ne!(IsWindowEnabled(windows.owner), 0);
        let account = updates.try_recv().unwrap();
        assert_eq!(account.config.provider, OAuthProvider::Google);
        assert_eq!(account.config.client_id, "google-desktop-client");
        assert!(!account.signed_in());
        drop(windows);
        let (windows, updates) = show_fixture(Some(signed_google()));
        assert_eq!(
            text(GetDlgItem(windows.oauth, CLIENT_SECRET as i32)),
            "",
            "Saved secrets must never be filled into an edit"
        );
        SendMessageW(windows.oauth, WM_COMMAND, SIGN_OUT as usize, 0);
        SendMessageW(windows.oauth, WM_COMMAND, IDCANCEL as usize, 0);
        assert!(
            updates.try_recv().is_err(),
            "Cancel must discard sign-out and all configuration changes"
        );
    }
}

#[test]
fn changing_custom_token_endpoint_never_transfers_the_saved_secret_to_a_new_origin() {
    let mut original = signed_google();
    original.config.provider = OAuthProvider::Custom;
    let mut fields = input(&original);
    fields.token_endpoint = "https://different-provider.example/token".into();
    let account = fields.account(&original).unwrap();
    assert!(!account.config.has_client_secret());
    assert!(account.tokens.is_none());
}
