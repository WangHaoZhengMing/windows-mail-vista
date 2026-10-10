// Compile from the repository root: typst compile README.typ output/pdf/README.pdf
// All interface illustrations below are editable CeTZ vectors, with no image assets.
#import "@preview/cetz:0.5.2": canvas, draw

#let ink = rgb("24384B")
#let blue = rgb("235C91")
#let pale = rgb("EDF4FA")
#let edge = rgb("C7D4DF")
#let muted = rgb("617488")
#set document(title: "Windows Mail Vista · 项目说明", author: "Windows Mail Vista project")
#set page(
  paper: "a4",
  margin: (x: 20mm, top: 18mm, bottom: 18mm),
  header: context { if counter(page).get().first() > 1 { text(8pt, fill: muted)[Windows Mail Vista / 项目说明] } },
  footer: context align(right, text(8pt, fill: muted)[#counter(page).display("1")]),
)
#set text(font: ("Microsoft YaHei", "Segoe UI"), size: 9.5pt, fill: ink, lang: "zh")
#set par(leading: 0.65em, justify: false)
#set heading(numbering: none)
#show heading.where(level: 1): it => block(above: 17pt, below: 9pt, text(18pt, weight: "bold", fill: blue, it.body))
#show heading.where(level: 2): it => block(above: 12pt, below: 7pt, text(12pt, weight: "bold", it.body))
#show link: set text(fill: blue)
#show raw.where(block: true): it => block(width: 100%, inset: 10pt, fill: pale, radius: 4pt, breakable: true, text(
  font: "Consolas",
  size: 8pt,
  it,
))
#show raw: set text(font: "Consolas", size: 8pt)
#set table(stroke: 0.4pt + edge, inset: 6pt)
#show table.cell.where(y: 0): set text(weight: "bold")
#set table(fill: (x, y) => if y == 0 { pale } else { none })
#set figure(gap: 7pt, supplement: [图])
#show figure.caption: set text(size: 8pt, fill: muted)

// Drawing coordinates use centimetres. Every control is a CeTZ shape or text.
#let ui-label(x, y, body, size: 8pt, color: ink, weight: "regular", anchor: "west") = draw.content(
  (x, y),
  text(font: ("Segoe UI", "Microsoft YaHei"), size: size, fill: color, weight: weight, body),
  anchor: anchor,
)
#let panel(x, y, w, h, color: white, border: edge) = draw.rect(
  (x, y),
  (x + w, y + h),
  fill: color,
  stroke: 0.45pt + border,
)
#let button(x, y, w, label, primary: false) = {
  panel(x, y, w, 0.42, color: if primary { pale } else { rgb("FAFBFC") }, border: if primary { blue } else { edge })
  ui-label(x + w / 2, y + 0.21, label, size: 7.5pt, anchor: "center")
}
#let envelope(x, y, color: muted) = {
  panel(x, y, 0.25, 0.17, border: color)
  draw.line((x, y + 0.17), (x + 0.125, y + 0.075), (x + 0.25, y + 0.17), stroke: 0.5pt + color)
}
#let folder(x, y) = {
  panel(x, y, 0.25, 0.18, color: rgb("F5D36F"), border: rgb("B89A44"))
  panel(x + 0.01, y + 0.18, 0.1, 0.055, color: rgb("F5D36F"), border: rgb("B89A44"))
}
#let window-frame(title, height) = {
  panel(0.045, -0.045, 16, height, color: rgb("E8EDF1"), border: rgb("E8EDF1"))
  panel(0, 0, 16, height)
  panel(0, height - 0.43, 16, 0.43, color: rgb("407B8A"), border: rgb("407B8A"))
  envelope(0.18, height - 0.29, color: white)
  ui-label(0.58, height - 0.22, title, color: white, size: 8.5pt)
  draw.line((15.65, height - 0.3), (15.8, height - 0.15), stroke: 0.7pt + white)
  draw.line((15.65, height - 0.15), (15.8, height - 0.3), stroke: 0.7pt + white)
}

#let mail-ui() = canvas({
  window-frame("Windows Mail Vista", 8.3)
  ui-label(0.2, 7.62, "File     Edit     View     Tools     Message     Help")
  panel(0, 6.98, 16, 0.44, color: blue, border: blue)
  for (x, label) in (
    (0.25, "Create Mail"),
    (2.25, "Reply"),
    (3.55, "Reply All"),
    (5.3, "Forward"),
    (7.0, "Send/Receive"),
    (9.65, "Stop sync"),
  ) {
    envelope(x, 7.1, color: white)
    ui-label(x + 0.36, 7.2, label, color: white, size: 7pt)
  }
  panel(12.1, 7.51, 3.55, 0.27)
  ui-label(12.22, 7.65, "Search mail", color: muted, size: 7pt)
  panel(0, 0.36, 3.8, 6.6, color: rgb("F9FBFD"))
  panel(3.8, 0.36, 0.07, 6.6, color: pale)
  folder(0.2, 6.5)
  ui-label(0.55, 6.61, "demo@example.com", size: 7.5pt)
  for (i, label) in ("Inbox", "Outbox", "Sent Items", "Drafts", "Deleted Items", "Junk", "Recovered").enumerate() {
    let y = 6.12 - i * 0.35
    if i == 0 { panel(0.45, y - 0.1, 3.05, 0.31, color: rgb("DDEBFA"), border: rgb("DDEBFA")) }
    folder(0.55, y)
    ui-label(0.9, y + 0.1, label, size: 7.5pt)
  }
  folder(0.2, 3.39)
  ui-label(0.55, 3.5, "Local Folders", size: 7.5pt)
  for (i, label) in ("Inbox", "Drafts", "Recovered").enumerate() {
    folder(0.55, 3.04 - i * 0.35)
    ui-label(0.9, 3.14 - i * 0.35, label, size: 7.5pt)
  }
  panel(3.88, 6.53, 12.12, 0.44, color: pale)
  ui-label(4.02, 6.76, "!")
  ui-label(4.7, 6.76, "Flag")
  ui-label(5.4, 6.76, "From")
  ui-label(8.95, 6.76, "Subject")
  ui-label(14.15, 6.76, "Received")
  for (i, row) in (
    ("Service updates", "Scheduled maintenance", "10 Oct"),
    ("Example team", "Welcome to Windows Mail Vista", "08 Oct"),
    ("Project notes", "Your weekly summary", "07 Oct"),
    ("Travel desk", "Booking confirmation", "28 Sep"),
    ("Alex", "Re: Meeting notes", "25 Sep"),
    ("Release list", "New version available", "21 Sep"),
  ).enumerate() {
    let y = 6.2 - i * 0.42
    if i == 1 { panel(3.89, y - 0.13, 12.1, 0.41, color: rgb("E5EFF8"), border: rgb("E5EFF8")) }
    envelope(4.05, y - 0.02)
    ui-label(5.4, y + 0.08, row.at(0), size: 7.3pt)
    ui-label(8.95, y + 0.08, row.at(1), size: 7.3pt)
    ui-label(14.15, y + 0.08, row.at(2), size: 7.3pt)
  }
  panel(3.88, 3.0, 12.12, 0.4, color: rgb("F0F2F4"))
  ui-label(4.06, 3.2, "From: Example team    To: demo@example.com", size: 7pt)
  ui-label(4.08, 2.62, "Welcome to Windows Mail Vista", size: 10pt, color: blue, weight: "bold")
  ui-label(4.08, 2.18, "This is the reading pane. Select a message to read its content.", size: 7.5pt)
  ui-label(4.08, 1.79, "HTML and attachments are available in the message window.", size: 7.5pt)
  panel(0, 0, 16, 0.36, color: rgb("F0F2F4"))
  ui-label(0.18, 0.18, "142 messages, 112 unread", size: 7pt)
  ui-label(4.75, 0.18, "Sync: 2   /   INBOX: downloaded 16 new messages", size: 7pt, color: blue)
})

#let settings-ui() = canvas({
  window-frame("Settings - Windows Mail Vista", 6.8)
  panel(0, 0.65, 4.2, 5.71, color: pale, border: pale)
  panel(0.25, 5.35, 3.65, 0.62, color: rgb("CBE2F5"), border: rgb("CBE2F5"))
  panel(0.25, 5.35, 0.06, 0.62, color: blue, border: blue)
  for (i, label) in ("Appearance", "Accounts", "Storage", "About").enumerate() {
    ui-label(0.62, 5.66 - i * 0.66, label, color: if i == 0 { blue } else { muted }, size: 9pt)
  }
  ui-label(4.66, 5.72, "Appearance & layout", size: 11pt, weight: "bold", color: blue)
  draw.line((4.65, 5.33), (15.45, 5.33), stroke: 0.5pt + edge)
  ui-label(4.66, 4.97, "Choose the panes shown in the main window.", size: 8.5pt)
  for (y, label) in ((4.12, "Show folder sidebar"), (3.42, "Show message preview")) {
    panel(4.68, y - 0.09, 0.23, 0.23, color: blue, border: blue)
    draw.line((4.72, y + 0.01), (4.78, y - 0.04), (4.87, y + 0.09), stroke: 0.8pt + white)
    ui-label(5.04, y + 0.04, label, size: 9pt)
  }
  ui-label(4.66, 2.66, "Changes apply when you select Save changes.", size: 8pt, color: muted)
  ui-label(4.66, 2.25, "Your choices are remembered for the next launch.", size: 8pt, color: muted)
  panel(0, 0, 16, 0.65, color: rgb("F0F2F4"))
  button(10.75, 0.12, 2.6, "Save changes", primary: true)
  button(13.65, 0.12, 1.95, "Cancel")
})

#let wizard-ui() = canvas({
  window-frame("New Account Wizard - Windows Mail Vista", 7.0)
  panel(0, 5.16, 16, 1.4, color: pale, border: pale)
  envelope(0.55, 5.8, color: blue)
  ui-label(1.1, 6.02, "Choose your email service", size: 11pt, weight: "bold", color: blue)
  ui-label(1.1, 5.57, "Step 1 of 3  /  Service → Identity → Review", size: 8pt, color: muted)
  ui-label(0.55, 4.72, "Server and encryption settings will be filled in for you.", size: 8.5pt)
  panel(0.55, 1.57, 14.9, 2.8)
  for (i, label) in (
    "iCloud Mail",
    "Gmail / Google Workspace",
    "Outlook.com / Hotmail",
    "Yahoo Mail",
    "QQ Mail",
    "Other / manual setup",
  ).enumerate() {
    let y = 4.05 - i * 0.43
    if i == 0 { panel(0.57, y - 0.16, 14.86, 0.42, color: rgb("DDEBFA"), border: rgb("DDEBFA")) }
    envelope(0.83, y - 0.02, color: blue)
    ui-label(1.25, y + 0.07, label, size: 8pt)
  }
  ui-label(0.55, 1.13, "Review your server settings and use Test connection before saving.", size: 8pt, color: muted)
  panel(0, 0, 16, 0.66, color: rgb("F0F2F4"))
  button(9.32, 0.13, 1.7, "‹ Back")
  button(11.28, 0.13, 1.8, "Next ›", primary: true)
  button(13.34, 0.13, 2.12, "Cancel")
})

#text(9pt, fill: muted, tracking: 1.5pt)[RUST DESKTOP MAIL CLIENT]
#v(5pt)
#text(32pt, fill: blue, weight: "bold")[Windows Mail Vista]
#v(4pt)
#text(12pt, fill: muted)[项目说明 · 使用指南 · 开发入口]
#v(12pt)
#figure(mail-ui(), caption: [主窗口结构。所有控件与图标由 CeTZ 绘制；内容为演示数据，布局为示意。])
#v(5pt)


一个使用 Rust 编写的 Windows 桌面邮件客户端。主界面保留经典 Windows Mail 的菜单、蓝色工具栏和文件夹布局，设置与账户配置使用独立窗口。

目前可运行的平台是 *Windows x64*。macOS 和 Linux 仅有预留入口与适配目录，尚不能作为完整客户端使用。本项目是独立实现，并未完整复刻旧 Windows Mail 的所有功能和接口。

#pagebreak(weak: true)
= 快速开始

== 构建环境

- Windows 10 1703 或更新版本，或 Windows 11；x64。
- Rust 1.88 或更新版本，使用 `x86_64-pc-windows-msvc` 工具链。
- Visual Studio Build Tools 的 C++ 构建工具及 Windows SDK，包含 `rc.exe`。

在项目根目录执行：

```powershell
cargo build --release --locked
.\target\release\WinMail.exe
```

首次构建需要下载 Cargo 依赖。依赖已缓存后，可添加 `--offline` 离线构建。

开发时直接运行：

```powershell
cargo run -p mail_windows --bin WinMail
```

构建产物：

#table(
  columns: (auto, 1fr),
  table.header([文件], [用途]),
  [`target/release/WinMail.exe`], [Windows 图形客户端],
  [`target/release/winmail-rust.exe`], [命令行工具],
  [`target/release/msoe.dll`], [单独构建的可选旧接口兼容库],
)

图标和清单嵌入 EXE，无需随程序复制 `assets`。正常 GUI 启动直接调用 Rust 界面。默认 HTML 阅读后端使用系统 MSHTML，以获得最佳性能；显式使用 WebView2 时需要对应运行环境。

#pagebreak(weak: true)
= 使用界面

== 添加账户
#figure(wizard-ui(), caption: [账户新建向导。选择服务商、填写身份信息，再审核服务器设置。])

打开 *Tools → Accounts... → Add...*，进入账户新建向导：

+ 选择服务商：iCloud、Gmail / Google Workspace、Outlook.com / Hotmail、Yahoo、QQ，或其他邮箱。
+ 填写邮箱地址和可选账户名称。
+ 审核预填的服务器、端口和加密方式，填写登录信息；点击 *OK* 保存。

预设使用 IMAP，并允许在最后一步修改配置。其他邮箱可手动填写服务器，也可选择 POP3。在向导最后一步或已有账户的属性中，点击 *Test connection*，可在后台测试收件与 SMTP 服务器的连接、加密协商和登录，分别显示结果。测试不会发送或下载邮件，也不会保存表单；确认配置后点击 *OK* 保存。OAuth 账户需先完成授权。

#table(
  columns: (auto, 1fr),
  table.header([服务商], [登录提示]),
  [iCloud], [使用 Apple 应用专用密码],
  [Gmail / Google Workspace], [使用可用的应用密码，或在属性中配置 OAuth；组织策略可能限制访问],
  [Outlook.com / Hotmail], [启用 IMAP，使用 OAuth 2.0],
  [Yahoo], [使用 Yahoo 应用密码],
  [QQ], [启用 IMAP/SMTP，使用授权码],
)

Microsoft、Google 和自定义 OAuth 支持系统浏览器登录、PKCE 和令牌刷新，但需要自行注册应用并配置 Client ID。项目没有内置可直接使用的服务商应用 ID。参见 #link("analysis/OAUTH.md")[ OAuth 配置 ] 和 #link("app/windows/src/ui/account_window/providers.md")[ 服务商预设与官方资料 ]。

== 收发与整理邮件

- 启动后自动在后台同步已配置的账户，最多同时同步 3 个不同账户；*Send/Receive* 手动同步当前账户。同一账户不会重复启动同步。底部状态栏显示同步任务数量、当前账户、文件夹进度、下载邮件数和数据量。
- 工具栏的 *Stop sync* 或 *Tools → Stop synchronization* 停止正在运行和排队的同步。连接建立、域名解析或 OAuth 刷新期间，停止可能需要等待该步骤返回；已建立的邮件连接会关闭以打断等待。
- IMAP 和 POP3 每封邮件完整下载后，立即通过原子写入保存到数据目录的 `sync-checkpoints`。停止、断网或重启后，再次同步会核对服务器标识并复用这些内容，成功提交邮件目录后移除对应检查点。邮件列表仍在批次提交后更新，未下载完整的一封邮件会重新下载；目前不支持单封邮件的字节级续传。
- *Create Mail* 新建邮件，写信窗口的 *From* 选择发件账户。
- 双击邮件或按 Enter 打开独立阅读窗口；支持回复、回复全部和转发。
- 搜索框搜索主题和发件人；筛选器区分已读与未读，点击列表列头排序。
- 列表发件人优先显示昵称，悬停查看完整地址；收件时间按电脑本地时区显示今天的时分、昨天的时分或月日，悬停查看完整日期。未读邮件使用闭合信封，发件人和主题加粗；已读邮件使用打开信封。搜索和日期排序仍使用原始邮件信息。
- 支持草稿、附件、EML 导入与导出，以及删除、恢复、永久删除和清空垃圾箱。
- 拖动侧栏与右侧内容之间的分隔条调整宽度；预览区也支持调整大小。

切换主窗口账户不会改变已经打开的写信窗口的发件账户。

*Local Folders* 是独立于邮箱账户的本地存储分组，用于导入邮件以及保留移除账户后的邮件和草稿。账户邮件也会缓存到电脑，本地分组与账户分组的区别是邮件归属。

== 设置与关于
#figure(settings-ui(), caption: [设置窗口。左侧导航、布局开关和底部操作区均为矢量绘制。])

打开 *Tools → Settings...*：

#table(
  columns: (auto, 1fr),
  table.header([分类], [当前功能]),
  [Appearance], [显示或隐藏文件夹侧栏、邮件预览窗格；保存后立即应用，下次启动恢复],
  [Accounts], [打开账户管理窗口],
  [Storage], [查看本地数据目录和 Local Folders 说明],
  [About], [查看版本、功能介绍和项目说明],
)

设置窗口目前没有主题切换、自动收取间隔等选项。布局偏好保存在数据目录的 `desktop-settings.json`。

== HTML 阅读

默认使用 MSHTML / Trident，另有 WebView2 后端。支持 HTML 正文、静态 CSS、表格和 CID 内嵌图片；初始化失败时回退为纯文本。

远程图片默认阻止，可通过当前邮件的 *Show pictures* 加载；切换邮件后重新阻止。邮件脚本与活动内容禁用。实现边界见 #link("analysis/HTML.md")[ HTML 阅读说明 ]。

#pagebreak(weak: true)
= 数据与命令行

默认数据目录为 `%LOCALAPPDATA%\WinMailRust`。可在启动前指定其他目录：

```powershell
$env:WINMAIL_DATA_DIR = 'D:\MailData\Rust'
.\target\release\WinMail.exe
```

目录中保存原始 EML、邮件索引、草稿、附件副本、多账户配置和界面偏好。密码与 OAuth 凭据由当前 Windows 用户的 DPAPI 加密；邮件正文和草稿内容未加密。备份应包含整个数据目录，DPAPI 凭据不能假定可直接跨用户或跨电脑使用。

同一数据目录使用进程锁。GUI 打开时，对该目录执行 CLI 或 MAPI 存储操作可能报告正在使用。正常 GUI 再次启动会向现有实例转发命令。

CLI 示例：

```powershell
cargo run -p mail_cli -- --help
.\target\release\winmail-rust.exe --mail-import 'D:\ExportedMail'
.\target\release\winmail-rust.exe --mail-list
.\target\release\winmail-rust.exe --mail-show MESSAGE_ID
.\target\release\winmail-rust.exe --mail-export MESSAGE_ID 'D:\sample.eml'
.\target\release\winmail-rust.exe --mail-receive
.\target\release\winmail-rust.exe --mail-draft 'D:\draft.json'
```

`MESSAGE_ID` 使用列表输出的邮件 ID。导入支持递归读取 `.eml` / `.nws`，不支持旧 ESE / DBX 数据库。

草稿 JSON 示例：

```json
{
  "to": "recipient@example.com",
  "cc": "",
  "bcc": "",
  "subject": "Hello",
  "body": "Message text",
  "attachments": []
}
```

实际发送邮件：

```powershell
.\target\release\winmail-rust.exe --mail-send 'D:\draft.json'
```

GUI 也可从 `mailto:` 或 EML 文件启动：

```powershell
.\target\release\WinMail.exe 'mailto:someone@example.com?subject=Hello'
.\target\release\WinMail.exe '/eml:C:\Mail\sample.eml'
```

更多命令见 #link("app/cli/README.md")[ CLI 说明 ]。同步、删除与恢复的具体行为见 #link("analysis/IMAP.md")[ IMAP 说明 ]。

#pagebreak(weak: true)
= 代码结构

```text
app/
  cli/                        命令行入口
  windows/
    src/ui/                   主窗口、阅读、写信、设置与账户向导
    src/startup/              参数路由、单实例、系统初始化及旧启动支持
    platform/                 Windows 系统服务与控件
      src/icons/
        mod.rs                图标公共接口、ICO 与图像列表
        catalog.rs            图标名称与文件映射
        bitmap.rs             PNG 解码、透明度与 DPI 缩放
    build_support/            EXE 图标与 manifest 嵌入
    compat/                   可选 msoe.dll / Simple MAPI 适配
  macos/                      预留平台入口与适配目录
  linux/                      预留平台入口与适配目录
crates/
  mail_core/                  账户、MIME、存储、协议与 OAuth
  mail_app/                   应用状态与邮件操作
  mail_html/                  共享 HTML 渲染接口
assets/windows/               Windows 图标、清单与设计原稿
analysis/                     架构、协议、兼容范围及历史分析资料
```

Cargo workspace 统一版本、依赖和 lint；默认构建 Windows 与 CLI 两个入口。共享业务不依赖应用入口。维护边界见 #link("analysis/ARCHITECTURE.md")[ 架构说明 ]。

== 图标管理

程序使用独立的 *128×128 RGBA PNG*，按用途命名：

- `assets/windows/icons/toolbar/`：compose、reply、forward、attach 等工具栏图标。
- `assets/windows/icons/folders/`：account、inbox、drafts、trash 等树图标。
- `assets/windows/icons/message-status/`：read、unread。
- `assets/windows/icons/app.ico`：多尺寸程序图标。

界面通过 `ToolbarIcon`、`TreeIcon`、`FolderIcon` 使用图标。统一清单生成 Windows 图像列表顺序，界面无需维护大图集坐标。

替换图标：修改对应 PNG 后重新构建。新增图标：添加 PNG，在 `catalog.rs` 登记名称，再在界面选择对应枚举。旧图集保留于 `sources/legacy - runtime-atlases/`，仅作设计记录与测试参照，不嵌入正式程序。详见 #link("assets/windows/README.md")[ Windows 资源说明 ]。

#pagebreak(weak: true)
= 兼容范围与未实现功能

`app/windows/compat` 提供旧 Windows Mail ABI 和部分 Simple MAPI 适配。按需构建：

```powershell
cargo build -p mail_windows_compat --release --locked
```

该 DLL 只提供已实现的导出子集，不能作为完整系统 `msoe.dll` 的替代品。普通客户端不需要它。`WINMAIL_RUNTIME_DIR` 用于显式选择外部旧 DLL，日常使用无需设置。

当前未实现：

- 完整旧 COM、Extended MAPI、BMAPI 和旧数据库／身份迁移。
- 打印、通讯录、日历、拼写检查与富文本写信。
- 邮件规则、签名、全文搜索和 NNTP 新闻组。
- 可用的 macOS / Linux 原生客户端。

项目提供服务商预设和协议实现，但不代表已对所有真实服务商账户完成验证。完整边界见 #link("analysis/IMPLEMENTATION.md")[ 实现说明 ] 和 #link("app/windows/compat/README.md")[ 兼容模块说明 ]。

#pagebreak(weak: true)
= 开发验证

在项目根目录执行：

```powershell
cargo fmt --all -- --check
cargo check -p mail_windows --tests
cargo test -p mail_windows --lib -- --test-threads=1
cargo test -p mail_windows_platform --features ui -- --test-threads=1
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

上述命令是开发检查入口，不表示每次修改都已执行全部检查。Windows 原生窗口测试需要可用的 Windows 桌面环境；外部组件相关测试还取决于运行环境。

测试覆盖 MIME、存储、协议、凭据保护、窗口交互、账户操作、向导、设置和图标缩放。GUI 集成测试在 `app/windows/tests/`，CLI 测试在 `app/cli/tests/`，兼容接口测试在 `app/windows/compat/tests/`。

进一步阅读：

- #link("analysis/UI.md")[ 界面实现记录 ]
- #link("app/windows/src/startup/README.md")[ 启动层说明 ]
- #link("analysis/OAUTH.md")[ OAuth 配置 ]
- #link("analysis/IMAP.md")[ IMAP 同步与删除行为 ]
- #link("analysis/RECONSTRUCTION.md")[ 旧程序分析记录 ]

== Typst 文档编译

`README.typ` 是本文档的可编辑源文件；`README.md` 保留为仓库首页说明。在项目根目录执行：

```powershell
typst compile README.typ output/pdf/README.pdf
```

本文档使用 CeTZ 0.5.2，已在 Typst 0.15.1 下编译验证。首次编译需要下载该包，后续可使用本地缓存。中文排版使用 Microsoft YaHei，代码使用 Consolas。

主窗口、设置页和账户向导分别由 `mail-ui`、`settings-ui` 和 `wizard-ui` 绘制，复用按钮、信封和文件夹等绘图函数。插图不依赖截图或外部图片，修改源文件即可调整文字、颜色和布局。它们用于说明界面结构，示例邮件为演示数据。
