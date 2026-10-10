# macOS platform services

`mail_macos_platform` reserves native macOS services and a WKWebView adapter.
`mail_core` selects its services through a private platform facade. Shared HTML
contracts come from `crates/mail_html`.

The default data-directory convention is present. Keychain credentials, store
locks, and native rendering remain unimplemented and return explicit errors.
