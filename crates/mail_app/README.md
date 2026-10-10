# mail_app

应用状态、类型化命令、过滤排序、邮件选择与收发操作。公共接口使用
`mail_core` 的邮件类型，不接收窗口句柄或原生命令编号。各 UI 后端负责
把用户操作转换为命令，并呈现 `Selection` 与 `MailCompletion`。

`AppState` 拥有存储和后台任务。`sort()` 返回当前排序的副本，便于 UI
显示排序状态；窗口无需保存另一套排序规则。

代码不依赖 Windows UI，但运行涉及存储的操作仍需要 Core 所选的平台服务
提供有效的存储锁和凭据服务。目前只有 Windows 后端实现这些服务，
macOS/Linux 后端是占位实现。应用测试暂时仍需 Windows。

```powershell
cargo test -p mail_app --locked --offline
```
