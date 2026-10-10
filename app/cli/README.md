# WinMail CLI

`app/cli` 是命令行产品入口，生成 `winmail-rust`。邮件命令直接调用共享的
`mail_core`，Windows 启动与旧 DLL 检查使用 `mail_windows::startup`。共享 crate
不依赖这个应用包。

```powershell
cargo run -p mail_cli -- --help
cargo run -p mail_cli -- --check
```

`--version`、`--check` 与邮件命令使用 WinMail 的产品命名。旧的
`--runtime-version`、`--runtime-check` 参数继续作为别名。显式选择旧 DLL
时仍使用 `WINMAIL_RUNTIME_DIR` 环境变量，以保留已有启动协议。

macOS/Linux 可以进入相同的邮件命令接口，但存储锁与凭据服务尚未实现，
相关命令会报告不支持。平台占位目录不代表这些服务已可用。
