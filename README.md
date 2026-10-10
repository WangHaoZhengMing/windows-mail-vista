# Windows Mail：Rust 邮件客户端

项目根据本机 `WinMail.exe` / `msoe.dll` 的 x64 反汇编分析启动接口，在 Rust 中重建启动行为，并实现邮件引擎和原生 Windows 界面。**普通启动不需要原 Windows Mail 的任何 DLL。** 原安装目录没有被覆盖或修改。

项目按应用入口、共享功能和资产组织：

```text
app/
  cli/                  命令行入口，bin winmail-rust
  windows/
    src/ui/             Windows 窗口、菜单、布局
    src/startup/        Windows 启动、单实例、旧参数与 PE 检查
    platform/           Windows 凭据、锁、控件、DPI、HTML 后端
    compat/             可选 msoe.dll / Simple MAPI 兼容包
  macos/
    src/ui.rs           预留 macOS UI
    platform/           预留 Keychain、锁、WKWebView 后端
  linux/
    src/ui.rs           预留 Linux UI
    platform/           预留 Secret Service、锁、WebKitGTK 后端
crates/
  mail_core/            账户、MIME、存储、邮件协议
  mail_app/             应用状态、命令、邮件操作
  mail_html/            平台无关的 HTML 渲染接口和文档类型
assets/
  windows/              程序资源、设计源稿与原版参照
```

工具栏与文件夹/邮件状态的单图、条带与预览可由脚本重新生成，分别写入 `target/asset-exports/toolbar/`、`target/asset-exports/folders/`，不参与源码管理；主 PNG、设计原稿与参照保留在 `assets/`。

根目录是虚拟 Cargo workspace，统一版本、依赖和 lint。`app/` 拥有平台实现和入口，`crates/` 拥有共享业务与接口。`mail_core` 通过私有 `cfg` 选择器依赖对应平台的服务包，不依赖 UI 或应用入口；平台服务也不依赖邮件业务。维护边界和修改入口见 [代码结构说明](analysis/ARCHITECTURE.md)。macOS/Linux 目前只有预留目录和明确的未实现入口，没有原生 UI、凭据存储或存储锁实现，也未完成跨平台验证。

这是可运行的 Rust Windows 邮件客户端及部分兼容接口，没有完成原 `msoe.dll` 全部函数及行为的等价还原。旧 COM/BMAPI、新闻组、地址簿、账户迁移等边界见 [实现说明](analysis/IMPLEMENTATION.md)。

## 构建和启动

当前需要 Windows 10 1703+ / Windows 11 x64、Rust 1.88+、MSVC / Windows SDK（含资源编译器 rc.exe）。

```powershell
Set-Location C:\Users\HallM\Desktop\windows-mail
cargo build --release --locked --offline
.\target\release\WinMail.exe
```

也可以直接通过 Cargo 运行：

```powershell
cargo run -p mail_windows --bin WinMail
cargo run -p mail_cli --bin winmail-rust -- --help
```

默认构建成员是 Windows 入口和 CLI 入口，普通启动直接调用 Rust UI。兼容 DLL 是独立的可选产物，执行 `cargo build -p mail_windows_compat --release --locked --offline` 构建。输出路径仍为：

- `target\release\WinMail.exe`：Windows GUI，静态链接 Rust 邮件功能，单独复制此 EXE 可运行。
- `target\release\winmail-rust.exe`：命令行入口。
- `target\release\msoe.dll`：由 `app/windows/compat` 的 `mail_windows_compat` 生成，提供已实现的启动与部分 Simple MAPI 导出；普通启动无需加载它。

Windows 实现使用系统 API、Schannel TLS 和 DPAPI。支持多个账户，左侧按账户显示服务器实际文件夹、层级和特殊用途目录；Local Folders 保存本地导入邮件和删除账户配置后保留的邮件。**Tools → Accounts...** 提供账户列表、添加、属性、删除和默认账户；属性窗口分组配置地址、凭据、服务器、端口和加密。使用 **Send/Receive** 接收当前账户，写信窗口通过 **From** 选择发件账户，再使用 **Send** 发信。切换主窗口账户不会改变已打开写信窗口的发件人。

经典界面重建了菜单、蓝色工具栏、文件夹树、邮件报告列表、可调整预览区与独立写信窗口。支持 Per Monitor V2 High DPI：缩放变化时重建字体、控件布局与图标。主窗口/写信工具栏、文件夹树与邮件列表使用嵌入的高清透明 PNG 图集，按 DPI 从 128×128 源图缩放；程序图标使用最高 256×256 的原版 ICO。资产按用途集中存放，见 [资产说明](assets/README.md)。搜索框位于白色菜单行右侧，已读筛选位于下方蓝色工具栏右侧；搜索框采用 Win7 风格的白色圆角边框、放大镜与清除按钮。预览 From / To / Subject 同行显示，仅标签加粗，长内容以省略号收尾。支持主题/发件人搜索、已读过滤和列排序；打印、通讯录、日历、拼写检查与富文本编辑尚未实现。详见 [界面还原记录](analysis/UI.md)。

HTML 阅读在 Windows 默认使用 MSHTML／Trident，保留可显式选择的 WebView2 后端；共享 `HtmlRenderer` 接口已预留 macOS WKWebView 与 Linux WebKitGTK 适配目录。MSHTML 不需要 Edge Runtime，初始化失败时显示纯文本回退。CID 内嵌图片默认显示，外部图片默认阻止，可在当前邮件点击 Show pictures；切换邮件后重新阻止。邮件脚本与活动内容始终禁用。原版证据、接口和兼容范围见 [HTML 说明](analysis/HTML.md)。

支持密码认证与 Microsoft、Google、自定义 OAuth 2.0，通过系统浏览器登录、PKCE 回环回调和 XOAUTH2 连接 IMAP/POP3/SMTP。账户属性中的 Authentication → OAuth settings... 配置 Client ID；需要自己的桌面应用注册，详见 [OAuth 配置](analysis/OAUTH.md)。令牌自动刷新，并先保存再开始邮件操作。对外连接要求 TLS/STARTTLS，Plain 仅用于回环测试；密码、OAuth 令牌与可选客户端凭据由当前 Windows 用户的 DPAPI 加密保存。

```powershell
.\target\release\WinMail.exe 'mailto:someone@example.com?subject=Hello'
.\target\release\WinMail.exe '/mailurl:mailto:someone@example.com?subject=Hello'
.\target\release\WinMail.exe '/eml:C:\mail folder\sample.eml'
```

支持含空格、中文和引号的参数，以及 `/mailurl:`、`/eml:`、`/mail`、`/mailonly`、`/outmail`。同一数据目录的第二个进程会把命令转发给现有窗口。

## 数据和命令行

默认数据目录是 `%LOCALAPPDATA%\WinMailRust`，保存原始 EML、JSON 索引、草稿字段、附件副本和多账户配置。旧单账户索引在打开时迁移，原始 EML 保留；账户具有稳定 ID，修改服务器或显示名称不改变邮件归属。邮件正文与草稿字段未加密。新客户端不读取或改变原 Windows Mail 的数据库、账户或邮件。

```powershell
$env:WINMAIL_DATA_DIR = 'D:\MailData\Rust'
.\target\release\winmail-rust.exe --mail-import 'D:\ExportedMail'
.\target\release\winmail-rust.exe --mail-list
.\target\release\winmail-rust.exe --mail-show MESSAGE_ID
.\target\release\winmail-rust.exe --mail-export MESSAGE_ID 'D:\sample.eml'
.\target\release\winmail-rust.exe --mail-receive
.\target\release\winmail-rust.exe --mail-draft 'D:\draft.json'
# 此命令向配置好的 SMTP 服务器实际提交邮件：
.\target\release\winmail-rust.exe --mail-send 'D:\draft.json'
```

`MESSAGE_ID` 是 `--mail-list` 输出的 64 位十六进制 ID。导入支持递归 `.eml` / `.nws`，不支持旧 ESE/DBX 数据库。草稿无需账户或完整收件人即可保存，发送前再验证。密送地址保存于草稿和 SMTP 信封，不写入发送 MIME 的 Bcc 头。

`D:\draft.json` 是自行创建的草稿文件，字段为 `to`、`cc`、`bcc`、`subject`、`body` 字符串与 `attachments` 文件路径数组。

单封最大 50 MiB，每批最多下载 200 封 / 100 MiB；GUI 和 CLI 在有进展、没有警告时自动继续最多 50 批，剩余邮件可再次 Send/Receive。IMAP 同步服务器所有可选择文件夹、层级与已读、旗标、回复等标记；离线修改已读和旗标会持久化，下一次同步提交。列表右键可移动到服务器文件夹。正文使用 BODY.PEEK[]，未读状态不会因下载改变。失败文件夹快照保留缓存；远端移除、UIDVALIDITY 或连接变化保留原始邮件到 Recovered。详见 [IMAP 同步说明](analysis/IMAP.md)。

删除先移入服务器 Deleted Items，支持恢复、确认后永久删除与清空垃圾箱。支持 MOVE 的服务器可直接移动；无 MOVE 时 COPY 后标记来源删除。没有 UIDPLUS 时保留“等待服务器清理”状态，实际确认 UID 消失后才移除缓存，始终避免全局 EXPUNGE 清除其他客户端的邮件。新 UID 无法可靠确认时保留正文并阻止重复远程操作。POP3 与本地导入邮件仅删除本地副本，不发送 POP3 DELE。失败发送保留 Outbox，不自动重试；SMTP DATA 获接受后认为提交成功，未实现最终投递确认。

索引使用临时文件替换，存储使用进程锁。损坏索引会报错并保留原文件；落盘但未索引的 EML 会恢复到 Recovered。窗口打开时，访问相同目录的 CLI 存储操作或 MAPI 会报“正在使用”，应先关闭窗口。

## 实现范围

| 功能 | 当前实现 |
| --- | --- |
| 原启动参数、策略、OLE、互斥、IPC、错误码和重启 | `app/windows/src/startup` 中的 Rust 实现 |
| 邮件窗口、阅读、回复/转发、草稿、附件、导入/导出 | Rust 原生 Win32 UI，经典主窗口、独立阅读窗口与写信窗口；双击/Enter 打开、右键操作邮件 |
| MIME、中文头、文本/HTML 预览、附件 | 保留 HTML 正文；统一 renderer trait，默认 MSHTML 显示表格、静态 CSS 和 CID 图片，失败回退文本 |
| EML/JSON 存储、重复下载识别、DPAPI 密码 | `mail_core` / `app/windows/platform` |
| SMTP PLAIN/LOGIN/XOAUTH2、完整 IMAP 文件夹/标记同步、POP3、TLS/STARTTLS | Rust 协议逻辑；Windows Schannel 验证证书 |
| Simple MAPI 登录、注销、查找、读取、保存、删除、地址解析、发送、释放缓冲区 | `mail_windows_compat` 已实现子集，保留对应原导出序号 |
| `/ident*` 和 ordinal 22 | Rust 存储/EML 识别，输出格式不同于旧实现 |
| `/reg`、`/unreg` | 保留注册表命令更新，不强制更改现代 UserChoice |
| 原 COM 类、Extended MAPI、BMAPI、MAPI 发送/地址簿对话框、Unicode MAPI | 未实现 |
| Microsoft / Google / 自定义 OAuth | 浏览器登录、PKCE、DPAPI、自动刷新，需注册应用 Client ID |
| 旧数据库/账户/身份迁移、NNTP、规则、签名、全文搜索 | 未实现 |
| OCInstall 旧安装维护 | 默认不支持；显式旧 DLL 模式可调用原程序 |
| wab.dll、wab32.dll、msidcrl30.dll 等其他旧组件 | 未逐项重写；新客户端不加载它们 |
| macOS/Linux 原生应用 | 仅预留目录与未实现入口 |

`msoe.dll` 不能直接替换系统目录里的完整旧 DLL：未提供所有旧导出和 COM 类，DllGetClassObject 明确返回 CLASS_E_CLASSNOTAVAILABLE。

对照旧核心时，显式设置 `WINMAIL_RUNTIME_DIR` 为原安装目录即可启用外部 DLL。普通 Rust 模式不设置此变量。

```powershell
$env:WINMAIL_RUNTIME_DIR = 'C:\Program Files\Windows Mail'
.\target\release\winmail-rust.exe --probe
# 恢复普通 Rust 模式：
Remove-Item Env:\WINMAIL_RUNTIME_DIR
```

## 验证和分析资料

```powershell
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo fmt --all -- --check
.\target\release\winmail-rust.exe --diagnose
.\target\release\winmail-rust.exe --probe
```

测试覆盖 MIME/中文/附件、存储恢复与锁、DPAPI、本地 SMTP/IMAP/POP3、STARTTLS 要求、TLS 失败、原生 DLL 调用、MAPI 内存释放和真实窗口的导入/回复/IPC/草稿恢复。CLI 进程测试在 `app/cli/tests/native_behavior.rs`，Windows 窗口集成测试在 `app/windows/tests/windows_behavior.rs`，MAPI 合同测试在 `app/windows/compat/tests/mapi_behavior.rs`，共享功能的测试随各 crate 保留。测试采用隔离目录，不读取用户邮件或向外部服务器发信。未验证真实服务商账户（包括 OAuth）、完整 TLS 证书链场景、旧客户端全行为兼容或 macOS/Linux 构建。

- [启动器反汇编记录](analysis/RECONSTRUCTION.md)
- [代码结构与维护入口](analysis/ARCHITECTURE.md)
- [邮件实现与 ABI 边界](analysis/IMPLEMENTATION.md)
- [本机输入文件哈希](analysis/inputs.json)
- [验证记录](analysis/validation.txt)

`scripts\Analyze.ps1` 可重新生成静态资料；`analysis\msoe.disasm.txt` 保存旧核心完整 LLVM 反汇编。这些资料不是恢复出的完整源码。
