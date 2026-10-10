# Windows 默认应用与 Simple MAPI

## 安装与默认应用

推荐直接下载 GitHub Release 的 MSI。双击安装到 `Program Files\Windows Mail Vista`，或静默执行：

```powershell
msiexec /i Windows-Mail-Vista-0.1.2-windows-x64.msi /qn /norestart
```

MSI 内含主程序、32/64 位 MAPI DLL 和开始菜单快捷方式，注册机器级默认应用候选项。安装不会更改现有默认邮箱。设置中的 **Accounts → Default mail app...** 用于选择 `mailto:` / EML 关联。

如果还需要旧软件通过系统 Simple MAPI 调用写信窗口，在管理员 PowerShell 中执行：

```powershell
& "$env:ProgramFiles\Windows Mail Vista\WinMail.exe" /register-mapi
```

MSI 支持版本升级、Windows Installer 修复及系统“已安装的应用”卸载。完整卸载会恢复通过 `/register-mapi` 保存的前一个系统 MAPI 客户端，保留用户邮件数据。手动注册的当前用户关联可先执行 `/unregister-app` 清理；其他用户的手动注册需要由该用户清理。

下面的脚本安装方式适用于从源码构建及开发测试。

在项目根目录构建：

```powershell
cargo build -p mail_windows -p mail_windows_compat --release --locked
```

仅安装当前用户的默认应用候选项：

```powershell
.\scripts\Install-SystemIntegration.ps1
```

程序复制到 `%LOCALAPPDATA%\Programs\Windows Mail Rust`，创建开始菜单快捷方式，并注册 `mailto:` 与 `.eml`。在软件 **Settings → Accounts → Default mail app...** 中打开 Windows 设置，选择相应的默认关联。注册不会强制覆盖 Windows 的 UserChoice。

点击邮件地址打开写信窗口；双击 EML 会导入 Local Folders 并打开阅读窗口。程序已运行时，通过现有 WM_COPYDATA 单实例机制处理请求。

## 启用系统 Simple MAPI

先构建 32 位适配 DLL，以支持 32 位调用软件：

```powershell
rustup target add i686-pc-windows-msvc
cargo build -p mail_windows_compat --target i686-pc-windows-msvc --release --locked
```

然后在 **管理员 PowerShell** 执行：

```powershell
.\scripts\Install-SystemIntegration.ps1 -EnableMapi
```

此模式安装到 `%ProgramFiles%\Windows Mail Rust`，根目录放 64 位 `msoe.dll`，`x86` 子目录放 32 位 `msoe.dll`。分别注册 64/32 位 `Software\Clients\Mail\Windows Mail Rust\DLLPath`，并将机器 Simple MAPI 默认客户端切换到本程序。保存原默认客户端用于撤销。

这一步和 `mailto:` 默认关联是不同的设置。两种位数的 DLL 都将交互写信转交给同一个 64 位 WinMail.exe，无需运行 32 位 GUI。

支持 `MAPISendMail`（ANSI）和 `MAPISendMailW`（Unicode）。设置 `MAPI_DIALOG` 时，填入收件人、抄送、密送、主题、正文和附件并打开写信窗口。调用线程等待结果，同时泵送 Windows 消息：

- SMTP 服务器确认接收邮件后返回 `SUCCESS_SUCCESS`（0）。这不表示收件人已经读到邮件。
- 关闭写信窗口返回 `MAPI_USER_ABORT`（1）。
- 无法启动程序、读取请求等返回失败。
- 网络发送失败时窗口保留，可重试或关闭；调用方继续等待用户处理。

附件先复制到当前用户临时请求目录，保留文件名；请求完成后清理。请求目录通过规范化路径验证。非零 MAPI 会话进入交互发送前释放其数据库锁；会话仍可注销，后续本地存储调用会尝试重新取得锁。如果 GUI 仍在使用该库，会返回邮件正在使用。

不带 `MAPI_DIALOG` 的发送保留原有直接 SMTP 路径，需要可用默认账户和数据库锁；GUI 已占用数据库时返回占用错误。没有提供完整 Extended MAPI、Outlook COM 自动化或地址簿实现，不能保证所有 Office/第三方调用方式都兼容。

## 撤销系统集成

普通当前用户安装：

```powershell
.\scripts\Unregister-SystemIntegration.ps1 -Destination "$env:LOCALAPPDATA\Programs\Windows Mail Rust"
```

机器 MAPI 安装，在管理员 PowerShell 执行：

```powershell
.\scripts\Unregister-SystemIntegration.ps1 -Destination "$env:ProgramFiles\Windows Mail Rust" -DisableMapi
```

仅当本程序仍是机器 MAPI 默认客户端时恢复原客户端，避免覆盖后来选择的软件。清理自己的注册项与快捷方式，保留程序文件和邮件数据。可在 Windows 默认应用设置中另选程序。

## 安装程序入口

`WinMail.exe /register-app`、`/unregister-app` 注册/移除当前用户候选项。

`WinMail.exe /register-mapi`、`/unregister-mapi` 设置/撤销机器 Simple MAPI 集成，需要管理员权限。安装时应先关闭旧版以避免 EXE/DLL 被锁定。普通启动不会写默认应用注册表。

`WINMAIL_EXECUTABLE` 可为适配调用显式指定主程序路径，主要用于开发和隔离测试。正式运行读取安装注册项。

目前交互适配实现模态 `MAPI_DIALOG`；显式 `MAPI_DIALOG_MODELESS` 返回不支持。没有通过提前返回成功来把未发送草稿报告为已发送。

隔离端到端取消测试（使用专用临时邮箱，不发送真实邮件）：

```powershell
cargo build -p mail_windows --bin WinMail --locked
$env:WINMAIL_EXECUTABLE = "$PWD\target\debug\WinMail.exe"
cargo test -p mail_windows_compat --test mapi_behavior interactive_mapi_opens_gui_and_returns_cancel -- --ignored --exact --test-threads=1
Remove-Item Env:WINMAIL_EXECUTABLE
```
