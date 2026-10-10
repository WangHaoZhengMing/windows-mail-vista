# Windows 资产

## 程序资源

| 文件 | 用途 |
| --- | --- |
| `icons/app.ico` | 原版多尺寸程序图标，含 256×256，供 EXE 构建与窗口标题图标使用。 |
| `icons/toolbar.png` | 1024×1024 透明 RGBA 图集，8×8 格、每格 128×128，按行保存索引 0–63，用于主窗口与写信窗口工具栏。 |
| `icons/folders.png` | 1280×768 透明 RGBA 图集，10×6 格、每格 128×128，按行保存 Folder / 邮件状态索引 0–59，用于文件夹树与邮件列表。 |
| `manifests/app.manifest` | PerMonitorV2 和 Common Controls 配置，由应用构建脚本嵌入。 |

`app/windows/platform` 嵌入并解码两套 PNG，每格独立缩放到实际 DPI 需要的尺寸，再以预乘 BGRA 提供给 Windows 图像列表。工具栏、文件夹树和邮件列表都使用高清源图。

图标按素材的实际含义选择：`FolderIcon::UnreadMessage` 使用闭信封 9，`ReadMessage` 使用开信封 10，文件夹树使用黄色文件夹 `Folder` 11。图集完整保留 60 个原始索引，其余图标可供后续对应状态使用。

## 原稿与参照

| 目录 / 文件 | 内容 |
| --- | --- |
| `originals/` | 原版 `toolbar.bmp`、`folders.bmp`，用于来源和视觉参照。 |
| `sources/toolbar/` | 当前 `source-atlas.png`、`prompt.txt` 提示词与制作说明。 |
| `sources/folders/source-atlas.png` | 当前修正后的 Folder / 邮件状态原稿，用于重新导出。 |
| `sources/folders/initial-atlas.png` | 初次生成原稿，保留作制作记录。 |
| `sources/folders/original-reference.png` | 用户提供的原始 960×16 图标条带。 |
| `sources/folders/reference-index.png` | 0–59 的编号参照。 |
| `sources/folders/prompt.txt`、`correction-prompt.txt` | 生成与修正提示词。 |

两套素材以原版图标为参照高清重绘，保留索引顺序、主要含义与经典配色，部分细节与原版不同。原版提取清单与工具栏索引参照继续位于 `analysis/ui-resources/`。

## 重新导出

在项目根目录执行：

```powershell
.\scripts\ExportToolbarHiDpi.ps1 -Source '.\assets\windows\sources\toolbar\source-atlas.png'
.\scripts\ExportFoldersHiDpi.ps1 -Source '.\assets\windows\sources\folders\source-atlas.png'
```

两类输出由独立参数控制：

| 参数 | 工具栏默认位置 | Folder 默认位置 | 生成内容 |
| --- | --- | --- | --- |
| `-Output` | `assets/windows/` | `assets/windows/` | 对应的程序主图集 `icons/toolbar.png` 或 `icons/folders.png`，以及对应 `sources/` 下的源稿副本。 |
| `-ExportOutput` | `target/asset-exports/toolbar/` | `target/asset-exports/folders/` | 独立图标、各尺寸 PNG/BMP 条带与预览；Folder 还生成主图集 `folders.png` 的辅助副本。 |

脚本允许源图已经位于目标设计源稿路径。隔离导出时同时指定两个参数：

```powershell
.\scripts\ExportToolbarHiDpi.ps1 -Source 'D:\toolbar-source.png' -Output '.\target\toolbar-assets' -ExportOutput '.\target\toolbar-exports'
.\scripts\ExportFoldersHiDpi.ps1 -Source 'D:\folders-source.png' -Output '.\target\folders-assets' -ExportOutput '.\target\folders-exports'
```

辅助导出是可重新生成的副本，保存在 `target/`，不参与源码管理：

| 导出文件 | 工具栏 | Folder / 邮件状态 |
| --- | --- | --- |
| `icons/` | `00.png`–`63.png`，每个 128×128 | `00.png`–`59.png`，每个 128×128 |
| `*-32.png` / `.bmp` | `toolbar-32`：2048×32 | `folders-32`：1920×32 |
| `*-48.png` / `.bmp` | `toolbar-48`：3072×48 | `folders-48`：2880×48 |
| `*-64.png` / `.bmp` | `toolbar-64`：4096×64 | `folders-64`：3840×64 |
| `*-128.png` / `.bmp` | `toolbar-128`：8192×128 | `folders-128`：7680×128 |
| `preview.png` | 768×768 浅色背景预览 | 960×576 浅色背景预览 |
| 主图集副本 | 程序主图保存在 `assets/windows/icons/toolbar.png` | 辅助输出 `folders.png`；程序使用 `assets/windows/icons/folders.png` |

PNG 保留 RGBA 透明度；BMP 为 24 位 RGB、白色背景，外部传统 image list 使用时需要白色遮罩。横向条带单格尺寸应匹配文件的 32、48、64 或 128 像素，索引保持工具栏 0–63、Folder 0–59。当前程序直接处理主 PNG 的 Alpha。

显示尺寸由逻辑尺寸与系统缩放决定。例如 16 个逻辑像素在 200% 缩放时需要 32 像素，300% 时需要 48 像素，400% 时需要 64 像素。导出只进行外部白底遮罩转换、裁切、居中与尺寸转换；封闭轮廓内的白纸和高光保持不透明。

运行程序不依赖原稿、参照或导出副本。更换主 PNG 后重新构建，更新图集才会进入 EXE / DLL；源稿、初始稿、提示词和编号参照继续保存。
