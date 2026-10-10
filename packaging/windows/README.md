# Windows MSI

发布产物是单个 x64 MSI，内嵌 CAB，不提供便携 ZIP。它包含 WinMail.exe、x64 msoe.dll 和 x86/msoe.dll。

```powershell
rustup target add x86_64-pc-windows-msvc i686-pc-windows-msvc
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build -p mail_windows -p mail_windows_compat --release --locked --target x86_64-pc-windows-msvc
cargo build -p mail_windows_compat --release --locked --target i686-pc-windows-msvc
./scripts/Build-Msi.ps1
```

构建脚本下载并校验固定版本的 WiX 3.14.1 工具，执行 MSI ICE 验证，输出 `dist/Windows-Mail-Vista-<version>-windows-x64.msi` 和 SHA-256 文件。可用 `-WixDirectory` 指定已有工具目录。

安装范围为整台机器，默认目录为 Program Files，需管理员权限。UpgradeCode 固定；版本必须使用数字 major.minor.patch，每次发布升级版本及 Git 标签。Windows Installer 管理文件、快捷方式和机器级关联的修复、回滚、卸载；不删除用户邮件，不强制设置 UserChoice。默认系统 MAPI 由用户显式启用，完整卸载时恢复已保存的前一客户端，升级时保留选择。

静默安装：`msiexec /i <installer.msi> /qn /norestart`。

GitHub workflow 推送版本标签后自动发布 MSI 和校验文件。GitHub 自带的源代码下载链接不是安装包。当前构建未配置代码签名；提交 Microsoft Store 前需要配置可信签名。
