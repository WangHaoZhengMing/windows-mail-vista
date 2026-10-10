# HTML mail contracts

This crate defines `HtmlRenderer`, `HtmlDocument`, `InlineImage`,
`RemoteImagePolicy`, and `HtmlStatus` without operating-system handles or
dependencies on native renderers.

Concrete HTML hosts live in `app/windows/platform`, `app/macos/platform`, and
`app/linux/platform`. Windows supplies MSHTML and WebView2; the other platforms
currently reserve adapter entry points.
