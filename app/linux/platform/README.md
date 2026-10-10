# Linux platform services

`mail_linux_platform` reserves native Linux services and a WebKitGTK adapter.
`mail_core` selects its services through a private platform facade. Shared HTML
contracts come from `crates/mail_html`.

The XDG data-directory convention is present. Secret Service credentials,
store locks, and native rendering remain unimplemented and return explicit
errors.
