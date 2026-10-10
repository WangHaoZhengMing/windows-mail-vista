# 项目资产

图片、图标与应用 manifest 集中放在根目录 `assets/`，应用入口和共享 crate 都从这里引用。

Windows 资产按用途分开，详细文件说明见 [windows/README.md](windows/README.md)：

- `windows/icons/`、`windows/manifests/`：程序使用的应用图标、工具栏/文件夹图集与 manifest。
- `windows/sources/`：设计原稿、提示词、初始生成稿和可重复导出的输入。
- `windows/originals/`：原版位图参照。

`ExportToolbarHiDpi.ps1` 和 `ExportFoldersHiDpi.ps1` 默认把主图集与源稿副本写到 `assets/windows/`，把可重新生成的单图、条带和预览分别写到 `target/asset-exports/toolbar/` 与 `target/asset-exports/folders/`。这些导出副本不参与源码管理。程序使用嵌入的透明 PNG，按 DPI 缩放；资产更新后需要重新构建程序。

`analysis/ui-resources/` 继续保存反汇编提取的来源证据，不承担当前程序资源目录的职责。
