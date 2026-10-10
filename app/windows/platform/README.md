# Windows platform services

`mail_windows_platform` owns Windows APIs, controls, DPI resources, icons, IPC,
and native HTML hosts. It does not depend on mail models or application state.

Default features expose only the data directory, DPAPI credential protection,
and exclusive store lock. `mail_core` selects those services through its private
platform facade. Enable `ui` for native controls and MSHTML/WebView2 renderers;
the Windows application enables this feature.

HTML renderer contracts come from `crates/mail_html`. Embedded artwork remains
in the repository's `assets/windows` directory.

Run native platform tests with:

```powershell
cargo test -p mail_windows_platform --features ui --lib
```
