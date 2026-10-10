# Windows 资源

## 程序使用的图标

每个图标都是独立的 128×128 RGBA PNG。替换图标时修改对应文件，重新构建即可，无需调整大图集。

- `icons/toolbar/`：15 个工具栏图标，例如 compose、reply、reply-all、forward、attach。
- `icons/folders/`：9 个树图标，例如 account、local、inbox、drafts、trash、junk。
- `icons/message-status/`：read、unread 两个邮件状态图标。
- `icons/app.ico`：多尺寸应用图标，供 EXE 和窗口使用。
- `manifests/app.manifest`：DPI 与 Common Controls 配置。

`app/windows/platform/src/icons/catalog.rs` 统一定义图标名称、文件和集合顺序。界面使用 ToolbarIcon、FolderIcon、TreeIcon，只有 Windows 控件边界将名称转换为索引。新增图标时在清单中添加名称与文件；集合长度自动计算。

`icons/bitmap.rs` 在首次使用时解码并缓存独立 PNG，逐个按 DPI 缩放为预乘 BGRA；`icons/mod.rs` 生成 Windows HIMAGELIST。程序不读取外部图片路径，不依赖图集坐标。

## 设计原稿

`sources/` 保留生成原稿、提示词及参考图片；`originals/` 保留原版 BMP。原来运行时使用的大图集已移到 `sources/legacy-runtime-atlases/`，仅作为设计记录和像素对照测试使用，不嵌入正式程序。

`ExportToolbarHiDpi.ps1` 和 `ExportFoldersHiDpi.ps1` 是旧图集导出工具，输出仅用于素材制作，不会更新独立图标。不要用它们替代命名 PNG 的维护流程。
