# Linux 应用入口

保留独立 Cargo package 与 `winmail-linux` 入口。当前 UI 后端未实现，运行会明确返回错误；尚未在 Linux 构建或验证。

本包的 `src/main.rs` 调用 `src/ui.rs`，`src/lib.rs` 提供可复用的应用入口。未来的原生窗口与界面留在本包，操作系统服务实现在相邻的 `platform/` 包中；账户、邮件、存储和同步等公共逻辑继续复用根目录 `crates/` 下的包。
