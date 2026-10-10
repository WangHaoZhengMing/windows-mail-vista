# Windows 应用

`mail_windows` 同时提供应用库与 `WinMail` 可执行入口。`src/ui/` 拥有窗口、菜单、布局和事件适配，`src/startup/` 拥有启动策略、单实例和历史参数。

底层系统服务位于 `platform/`，以独立叶子 package 避免与邮件引擎形成循环。应用启用它的 `ui` feature；Core 单独构建仅依赖路径、凭据与锁。共享邮件业务位于根目录 `crates/`。

`compat/` 的 `mail_windows_compat` 是可选的 `msoe.dll` / Simple MAPI 适配，普通 GUI 与 CLI 不依赖它。原生资源编译助手位于 `build_support/resources.rs`，与 CLI 共用；资产仍集中在 `assets/windows/`。

```powershell
cargo run -p mail_windows --bin WinMail
cargo test -p mail_windows --locked --offline
```

维护入口与资源生命周期见 [架构说明](../../analysis/ARCHITECTURE.md)。
