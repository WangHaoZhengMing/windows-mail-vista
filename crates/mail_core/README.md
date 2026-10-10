# mail_core

账户验证、MIME 解析和生成、EML/JSON 存储，以及 SMTP/IMAP/POP3 协议。
该 crate 不依赖 UI 或 Windows API；密码保护、默认数据目录和存储锁
通过私有 `platform.rs` 调用 `app/{windows,macos,linux}/platform` 中所选
目标的路径、凭据和锁服务。Windows 服务默认不启用 UI 依赖。

现有索引和账户字段保持兼容，包括 `password_dpapi` 这个存储键。
Windows 后端继续使用 DPAPI 和独占文件句柄。macOS/Linux 的凭据与
存储锁服务尚未实现，会返回明确错误，因此目前不能在这些平台使用
`Store::open()` 或账户密码保护。目录和模块边界已经准备好，完整的平台
支持尚未实现。

测试覆盖 MIME、草稿、持久化和回环服务器协议行为，目前涉及存储与
凭据的测试需 Windows。

```powershell
cargo test -p mail_core --locked --offline
```
