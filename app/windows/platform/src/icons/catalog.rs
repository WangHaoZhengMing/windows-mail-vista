//! Named embedded assets. Enum order is the private native image-list order.
macro_rules! icon_set {
    ($kind:ident, $assets:ident, $dir:literal, [$($variant:ident => $file:literal),+ $(,)?]) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        #[repr(i32)]
        pub enum $kind { $($variant),+ }
        pub(super) const $assets: &[(&str, &[u8])] = &[$(
            ($file, include_bytes!(concat!("../../../../../assets/windows/icons/", $dir, "/", $file, ".png")))
        ),+];
    };
}
icon_set!(ToolbarIcon, TOOLBAR, "toolbar", [
    Contacts => "contacts",
    Find => "find",
    Forward => "forward",
    Delete => "delete",
    SendReceive => "send-receive",
    Send => "send",
    NewMessage => "compose",
    Print => "print",
    Reply => "reply",
    ReplyAll => "reply-all",
    Save => "save",
    Cut => "cut",
    Copy => "copy",
    Paste => "paste",
    Attach => "attach",
    StopSync => "stop-sync",
]);
icon_set!(FolderIcon, MESSAGES, "message-status", [
    UnreadMessage => "unread",
    ReadMessage => "read",
]);
icon_set!(TreeIcon, FOLDERS, "folders", [
    Account => "account",
    Local => "local",
    Inbox => "inbox",
    Outbox => "outbox",
    Sent => "sent",
    Drafts => "drafts",
    Trash => "trash",
    Junk => "junk",
    Archive => "archive",
]);
