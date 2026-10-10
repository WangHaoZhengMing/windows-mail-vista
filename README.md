# Windows Mail Vista

排版文档见 [README.typ](README.typ)。其中的主窗口、设置页和账户向导使用 CeTZ 直接绘制，不依赖截图。执行 `typst compile README.typ output/pdf/README.pdf` 可生成 PDF。

Windows Mail Vista 是一个使用 Rust 编写的 Windows 桌面邮件客户端。主界面保留经典 Windows Mail 的菜单、蓝色工具栏和文件夹布局，设置与账户配置使用独立窗口。

目前可运行的平台是 **Windows x64**。macOS 和 Linux 仅有预留入口与适配目录，尚不能作为完整客户端使用。本项目是独立实现，并未完整复刻旧 Windows Mail 的所有功能和接口。

## Windows 自动构建与发布

GitHub Actions 配置位于 [windows-release.yml](.github/workflows/windows-release.yml)。

- 推送到 `main` 或提交 Pull Request：编译 Windows x64 版本，上传保留 14 天的构建产物。
- 推送版本标签（例如 `v0.1.0`）：测试、编译后自动创建 GitHub Release，附带便携 ZIP 和 SHA-256 校验文件。
- Actions → Windows build and release → Run workflow：不填 tag 时只构建；填已有版本标签可重新运行发布。

标签必须与根 `Cargo.toml` 的版本一致。发布新版本时先更新版本并提交，再执行：

```powershell
git tag v0.1.0
git push origin main
git push origin v0.1.0
```

下载 Release 压缩包，解压后运行 `WinMail.exe`。构建使用静态 C 运行库；可选 WebView 后端仍需系统安装 WebView2 Runtime。

## 快速开始

### 构建环境

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

| 文件 | 用途 |
| --- | --- |
| `target/release/WinMail.exe` | Windows 图形客户端 |
| `target/release/winmail-rust.exe` | 命令行工具 |
| `target/release/msoe.dll` | 单独构建的可选旧接口兼容库 |

图标和清单嵌入 EXE，无需随程序复制 `assets`。正常 GUI 启动直接调用 Rust 界面。默认 HTML 阅读后端使用系统 MSHTML，以获得最佳性能；显式使用 WebView2 时需要对应运行环境。

## 使用界面

### 添加账户

打开 **Tools → Accounts... → Add...**，进入账户新建向导：

1. 选择服务商：iCloud、Gmail / Google Workspace、Outlook.com / Hotmail、Yahoo、QQ，或其他邮箱。
2. 填写邮箱地址和可选账户名称。
3. 审核预填的服务器、端口和加密方式，填写登录信息；点击 **OK** 保存。

预设使用 IMAP，并允许在最后一步修改配置。其他邮箱可手动填写服务器，也可选择 POP3。在向导最后一步或已有账户的属性中，点击 **Test connection**，可在后台测试收件与 SMTP 服务器的连接、加密协商和登录，分别显示结果。测试不会发送或下载邮件，也不会保存表单；确认配置后点击 **OK** 保存。OAuth 账户需先完成授权。

| 服务商 | 登录提示 |
| --- | --- |
| iCloud | 使用 Apple 应用专用密码 |
| Gmail / Google Workspace | 使用可用的应用密码，或在属性中配置 OAuth；组织策略可能限制访问 |
| Outlook.com / Hotmail | 启用 IMAP，使用 OAuth 2.0 |
| Yahoo | 使用 Yahoo 应用密码 |
| QQ | 启用 IMAP/SMTP，使用授权码 |

Microsoft、Google 和自定义 OAuth 支持系统浏览器登录、PKCE 和令牌刷新，但需要自行注册应用并配置 Client ID。项目没有内置可直接使用的服务商应用 ID。参见 [OAuth 配置](analysis/OAUTH.md) 和 [服务商预设与官方资料](app/windows/src/ui/account_window/providers.md)。

### 收发与整理邮件

- 启动后自动在后台同步已配置的账户，最多同时同步 3 个不同账户；**Send/Receive** 手动同步当前账户。同一账户不会重复启动同步。底部状态栏显示同步任务数量、当前账户、文件夹进度、下载邮件数和数据量。
- 工具栏的 **Stop sync** 或 **Tools → Stop synchronization** 停止正在运行和排队的同步。连接建立、域名解析或 OAuth 刷新期间，停止可能需要等待该步骤返回；已建立的邮件连接会关闭以打断等待。
- IMAP 和 POP3 每封邮件完整下载后，立即通过原子写入保存到数据目录的 `sync-checkpoints`。停止、断网或重启后，再次同步会核对服务器标识并复用这些内容，成功提交邮件目录后移除对应检查点。邮件列表仍在批次提交后更新，未下载完整的一封邮件会重新下载；目前不支持单封邮件的字节级续传。
- **Create Mail** 新建邮件，写信窗口的 **From** 选择发件账户。
- 写信使用原生 RichEdit，支持字体、字号、粗体、斜体、下划线、文字颜色、段落对齐及项目符号。富文本发送包含纯文本和 HTML 两个版本；草稿另保留 RTF 编辑格式，重新打开可继续编辑。**Format** 菜单可选择纯文本或富文本输出，不启动 WebView。
- 双击邮件或按 Enter 打开独立阅读窗口；支持回复、回复全部和转发。
- 搜索框搜索主题和发件人；筛选器区分已读与未读，点击列表列头排序。
- 列表发件人优先显示昵称，悬停查看完整地址；收件时间按电脑本地时区和 Windows 区域格式显示，今天显示时间，昨天显示 Yesterday 加时间，今年的其他日期省略年份，往年保留年份。悬停可查看完整日期。未读邮件使用闭合信封，发件人和主题加粗；已读邮件使用打开信封。搜索和日期排序仍使用原始邮件信息。
- 支持草稿、附件、EML 导入与导出，以及删除、恢复、永久删除和清空垃圾箱。
- 拖动侧栏与右侧内容之间的分隔条调整宽度；预览区也支持调整大小。

切换主窗口账户不会改变已经打开的写信窗口的发件账户。

**Local Folders** 是独立于邮箱账户的本地存储分组，用于导入邮件以及保留移除账户后的邮件和草稿。账户邮件也会缓存到电脑，本地分组与账户分组的区别是邮件归属。

### 设置与关于

打开 **Tools → Settings...**：

| 分类 | 当前功能 |
| --- | --- |
| Appearance | 显示或隐藏文件夹侧栏、邮件预览窗格，切换 Use dark mode；保存后立即应用，下次启动恢复 |
| Accounts | 打开账户管理窗口 |
| Storage | 查看本地数据目录和 Local Folders 说明 |
| About | 查看版本、功能介绍和项目说明 |

深色模式调整应用窗口和原生控件的配色；邮件正文与撰写区域保留文档原有颜色。设置窗口目前没有自动收取间隔选项。布局和主题偏好保存在数据目录的 `desktop-settings.json`。

### HTML 阅读

默认使用 MSHTML / Trident，另有 WebView2 后端。支持 HTML 正文、静态 CSS、表格和 CID 内嵌图片；初始化失败时回退为纯文本。

独立阅读窗口工具栏提供 **Use WebView**，需要兼容复杂邮件时才创建 WebView2。点击 **Lightweight view** 切回 MSHTML，或关闭阅读窗口，即关闭 WebView2 控制器并释放宿主和相关对象，不缓存浏览器实例；初始化中的切换和关闭也会取消后续创建。浏览器进程退出由 WebView2 运行时管理，可能有短暂延迟。每封邮件重新打开时仍默认使用 MSHTML。

阅读窗口默认将邮件头压缩为主题及发件人昵称/日期两行；**Details** 展开或收起完整发件人、收件人和日期。**Copy text** 一键复制纯文本正文，不复制 HTML 标记，也无需先选择文字。

远程图片默认阻止，可通过当前邮件的 **Show pictures** 加载；切换邮件后重新阻止。邮件脚本与活动内容禁用。实现边界见 [HTML 阅读说明](analysis/HTML.md)。

## 数据与命令行

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

更多命令见 [CLI 说明](app/cli/README.md)。同步、删除与恢复的具体行为见 [IMAP 说明](analysis/IMAP.md)。

## 代码结构

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

Cargo workspace 统一版本、依赖和 lint；默认构建 Windows 与 CLI 两个入口。共享业务不依赖应用入口。维护边界见 [架构说明](analysis/ARCHITECTURE.md)。

### 图标管理

程序使用独立的 **128×128 RGBA PNG**，按用途命名：

- `assets/windows/icons/toolbar/`：compose、reply、forward、attach 等工具栏图标。
- `assets/windows/icons/folders/`：account、inbox、drafts、trash 等树图标。
- `assets/windows/icons/message-status/`：read、unread。
- `assets/windows/icons/app.ico`：多尺寸程序图标。

界面通过 `ToolbarIcon`、`TreeIcon`、`FolderIcon` 使用图标。统一清单生成 Windows 图像列表顺序，界面无需维护大图集坐标。

替换图标：修改对应 PNG 后重新构建。新增图标：添加 PNG，在 `catalog.rs` 登记名称，再在界面选择对应枚举。旧图集保留于 `sources/legacy-runtime-atlases/`，仅作设计记录与测试参照，不嵌入正式程序。详见 [Windows 资源说明](assets/windows/README.md)。

## 兼容范围与未实现功能

`app/windows/compat` 提供旧 Windows Mail ABI 和部分 Simple MAPI 适配。按需构建：

```powershell
cargo build -p mail_windows_compat --release --locked
```

该 DLL 只提供已实现的导出子集，不能作为完整系统 `msoe.dll` 的替代品。普通客户端不需要它。`WINMAIL_RUNTIME_DIR` 用于显式选择外部旧 DLL，日常使用无需设置。

当前未实现：

- 完整旧 COM、Extended MAPI、BMAPI 和旧数据库／身份迁移。
- 打印、通讯录、日历与拼写检查。
- 邮件规则、签名、全文搜索和 NNTP 新闻组。
- 可用的 macOS / Linux 原生客户端。

项目提供服务商预设和协议实现，但不代表已对所有真实服务商账户完成验证。完整边界见 [实现说明](analysis/IMPLEMENTATION.md) 和 [兼容模块说明](app/windows/compat/README.md)。

## 开发验证

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

- [界面实现记录](analysis/UI.md)
- [启动层说明](app/windows/src/startup/README.md)
- [OAuth 配置](analysis/OAUTH.md)
- [IMAP 同步与删除行为](analysis/IMAP.md)
- [旧程序分析记录](analysis/RECONSTRUCTION.md)
