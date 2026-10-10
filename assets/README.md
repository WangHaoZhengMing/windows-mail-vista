# 项目资源

程序图标、图片与 manifest 集中在根目录 `assets/`。

Windows 资源分为：

- `windows/icons/toolbar/`：按名称保存的独立工具栏 PNG。
- `windows/icons/folders/`：独立账户与文件夹 PNG。
- `windows/icons/message-status/`：独立邮件状态 PNG。
- `windows/icons/app.ico`、`windows/manifests/`：应用图标与清单。
- `windows/sources/`：设计原稿、提示词、参照与旧图集。
- `windows/originals/`：原版位图参照。

程序嵌入独立 PNG，通过统一清单管理名称，在运行时按 DPI 缩放并创建 Windows 图像列表。修改资源后需要重新构建。

旧图集导出脚本仅供素材制作，不是当前独立图标的更新入口。`analysis/ui-resources/` 保存历史提取证据。详细维护流程见 [Windows 资源说明](windows/README.md)。
