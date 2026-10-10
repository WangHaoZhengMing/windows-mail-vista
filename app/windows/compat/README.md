# Windows Mail compatibility

Windows Mail 的 ABI 与 Simple MAPI 兼容适配，库名保持 `msoe`，生成 `msoe.dll`。

支持 ANSI `MAPISendMail` 和 Unicode `MAPISendMailW` 的交互写信：请求转交给主程序，用户发送成功或取消后返回结果。提供 x64 / x86 构建，安装与默认客户端注册见 [系统集成](../SYSTEM-INTEGRATION.md)。无对话框发送仍需取得本地存储锁。

独立 package `mail_windows_compat` 处理外部调用约定、指针、返回码和 MAPI 缓冲区生命周期。账户、邮件、存储和协议实现在 `mail_core`，原生窗口由父目录 `mail_windows` library 提供。现有导出子集见 `msoe.def`；不提供完整旧 COM/MAPI 实现，也不参与正常 GUI 或 CLI 的启动依赖链。

兼容 DLL 不在默认构建成员中，需要时运行 `cargo build -p mail_windows_compat --release`。DLL 文件名仍为 `msoe.dll`，现有导出名称、序号和调用约定保持不变。Simple MAPI 测试在本 package 的 `tests/mapi_behavior.rs`。
