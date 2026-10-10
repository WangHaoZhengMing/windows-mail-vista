# Account wizard presets

Reviewed on 2026-10-10. Presets configure IMAP over TLS on port 993. SMTP uses STARTTLS on 587 or implicit TLS on 465. All fields remain editable during final review. The wizard does not test credentials or create accounts before the final properties form is saved.

| Provider | Incoming | Outgoing | Authentication guidance |
| --- | --- | --- | --- |
| iCloud | imap.mail.me.com:993 | smtp.mail.me.com:587 | Apple app-specific password. Full email address is used as the shared IMAP/SMTP username; Apple documents full-address IMAP fallback. |
| Gmail / Workspace | imap.gmail.com:993 | smtp.gmail.com:587 | App password when available with 2-Step Verification; OAuth may be configured in properties. Organization policies may restrict access. |
| Outlook.com / Hotmail | outlook.office365.com:993 | smtp-mail.outlook.com:587 | OAuth required; IMAP must be enabled. This application's existing OAuth flow requires a registered client ID. No application ID is invented or bundled by this wizard. |
| Yahoo | imap.mail.yahoo.com:993 | smtp.mail.yahoo.com:465 | Yahoo app password. |
| QQ | imap.qq.com:993 | smtp.qq.com:465 | Enable IMAP/SMTP and use the authorization code. |
| Other | User supplied | User supplied | Final properties form supports manual servers, POP3, security and OAuth configuration. |

Official references:

- [Apple iCloud server settings](https://support.apple.com/en-ie/102525)
- [Google IMAP and SMTP](https://developers.google.com/workspace/gmail/imap/imap-smtp)
- [Google client settings and app passwords](https://support.google.com/mail/answer/7104828)
- [Microsoft Outlook.com settings](https://support.microsoft.com/en-gb/outlook/pop-imap-and-smtp-settings-for-outlook-com)
- [Yahoo IMAP settings](https://in.help.yahoo.com/kb/SLN4075.html)
- [Tencent email connector documentation](https://main.qcloudimg.com/raw/document/product/pdf/1270_46586_cn.pdf)

The final properties form reuses existing account validation, password protection and save/cancel behavior. It is the third wizard step. OAuth configuration and browser sign-in remain explicit user actions within that form.
