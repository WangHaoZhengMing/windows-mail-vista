# Windows startup

CLI 和 Windows 应用共用的启动策略、命令路由、PE 文件检查和历史启动兼容接口。

`command` 与 `pe` 负责原始参数和 PE 检查。`platform::native` 负责单实例 IPC、启动策略、OLE、注册表操作和显式选择的旧 DLL 加载；普通启动直接调用同一 Windows library 的 `ui::run`。兼容 DLL 不参与普通启动，只有明确设置 `WINMAIL_RUNTIME_DIR` 时才加载外部 `msoe.dll`。
