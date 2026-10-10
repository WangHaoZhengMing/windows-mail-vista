# UI 模块

按窗口功能组织原生 Windows 界面。邮件业务逻辑、协议和存储分别由 `mail_app` 与 `mail_core` 提供。

| 目录或文件 | 职责 |
| --- | --- |
| `mod.rs` | 模块声明，导出 `run`、`active`、`shutdown` |
| `main/mod.rs` | 主窗口创建、DPI 更新及事件路由 |
| `main/view.rs` | 文件夹树、邮件列表、正文预览及操作状态 |
| `main/actions.rs` | 邮件命令、读信与写信窗口、账户变更 |
| `main/sync.rs` | 后台同步队列、进度及任务完成处理 |
| `main/dispatch.rs` | Win32 消息解码与重入事件队列 |
| `main/tests/` | 主窗口交互、布局、账户导航与服务器文件夹测试 |
| `reader/` | 独立阅读窗口、控件、菜单及测试 |
| `compose/` | 写信窗口及富文本编辑 |
| `accounts/` | 账户列表、属性、表单验证、服务商向导和 OAuth |
| `settings/` | 全局设置界面；`preferences.rs` 保存布局和主题偏好 |
| `setup/` | 首次启动向导，调用账户配置页 |
| `about.rs` | 关于窗口 |
| `shared/commands.rs` | 共用命令 ID 和窗口通知消息 |
| `shared/menus.rs` | 主窗口与写信窗口的菜单描述 |
| `shared/message_display.rs` | 阅读窗口与邮件列表共用的发件人和日期格式化 |
| `shared/window.rs` | 设置与账户窗口共用的 DPI 布局和窗口类注册 |

新增界面代码放入对应功能目录。窗口专用控件、菜单和测试跟随所属窗口；只有多个功能实际共用的代码才放入 `shared/`。全局设置不依赖账户模块，首次启动向导负责组合设置和账户流程。
