param(
    [Parameter(Mandatory = $true)][string]$Source,
    [string]$Output = (Join-Path $PSScriptRoot '../assets/windows'),
    [string]$ExportOutput = (Join-Path $PSScriptRoot '../target/asset-exports/toolbar')
)

# Package an ImageGen 8-by-8 atlas without changing its artwork or icon order.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing.Common, System.Drawing.Primitives, System.Collections, System.Runtime -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.Drawing;
public static class ToolbarMask {
    public static int[] Cuts(Bitmap image, bool horizontal) {
        int length = horizontal ? image.Height : image.Width;
        int cross = horizontal ? image.Width : image.Height;
        var gaps = new List<int>();
        int start = -1;
        for (int i = 0; i <= length; i++) {
            bool empty = i < length;
            if (empty) for (int j = 0; j < cross; j++) {
                Color p = horizontal ? image.GetPixel(j, i) : image.GetPixel(i, j);
                if (p.A > 0 && Math.Min(p.R, Math.Min(p.G, p.B)) < 210) { empty = false; break; }
            }
            if (empty && start < 0) start = i;
            if (!empty && start >= 0) {
                if (i - start >= 8) gaps.Add((start + i - 1) / 2);
                start = -1;
            }
        }
        if (gaps.Count != 9) throw new InvalidOperationException("Expected 8 separated icon rows/columns.");
        gaps[0] = 0; gaps[8] = length;
        return gaps.ToArray();
    }
    // Remove only exterior white connected to a cell boundary; enclosed white
    // paper and highlights remain opaque. This implements the legacy BMP mask.
    public static Rectangle Apply(Bitmap image) {
        int w = image.Width, h = image.Height;
        var visited = new bool[w * h];
        var queue = new Queue<int>();
        for (int x = 0; x < w; x++) { queue.Enqueue(x); queue.Enqueue((h - 1) * w + x); }
        for (int y = 0; y < h; y++) { queue.Enqueue(y * w); queue.Enqueue(y * w + w - 1); }
        while (queue.Count > 0) {
            int i = queue.Dequeue();
            if (visited[i]) continue;
            visited[i] = true;
            int x = i % w, y = i / w;
            Color p = image.GetPixel(x, y);
            if (p.A != 0 && (p.R < 240 || p.G < 240 || p.B < 240)) continue;
            image.SetPixel(x, y, Color.Transparent);
            if (x > 0) queue.Enqueue(i - 1);
            if (x + 1 < w) queue.Enqueue(i + 1);
            if (y > 0) queue.Enqueue(i - w);
            if (y + 1 < h) queue.Enqueue(i + w);
        }
        int left = w, top = h, right = -1, bottom = -1;
        for (int y = 0; y < h; y++) for (int x = 0; x < w; x++) {
            if (image.GetPixel(x, y).A == 0) continue;
            left = Math.Min(left, x); top = Math.Min(top, y);
            right = Math.Max(right, x); bottom = Math.Max(bottom, y);
        }
        if (right < left) throw new InvalidOperationException("Empty icon cell.");
        return Rectangle.FromLTRB(left, top, right + 1, bottom + 1);
    }
}
'@
$outputPath = [IO.Path]::GetFullPath($Output)
$iconPath = Join-Path $outputPath 'icons'
$sourcePath = Join-Path $outputPath 'sources/toolbar'
$exportPath = [IO.Path]::GetFullPath($ExportOutput)
$exportIconPath = Join-Path $exportPath 'icons'
foreach ($directory in @($iconPath, $sourcePath, $exportIconPath)) {
    [IO.Directory]::CreateDirectory($directory) | Out-Null
}
$inputPath = (Resolve-Path -LiteralPath $Source).Path
$sourceAtlasPath = Join-Path $sourcePath 'source-atlas.png'
$loadedAtlas = [Drawing.Bitmap]::new($inputPath)
try {
    $sourceIsPng = $loadedAtlas.RawFormat.Guid -eq [Drawing.Imaging.ImageFormat]::Png.Guid
    # Detach from the input file before writing any output. This also permits
    # regenerating the atlas when Source already points to icons/toolbar.png.
    $atlas = $loadedAtlas.Clone(
        [Drawing.Rectangle]::new(0, 0, $loadedAtlas.Width, $loadedAtlas.Height),
        [Drawing.Imaging.PixelFormat]::Format32bppArgb
    )
} finally { $loadedAtlas.Dispose() }
try {
    if (-not [string]::Equals($inputPath, $sourceAtlasPath, [StringComparison]::OrdinalIgnoreCase)) {
        if ($sourceIsPng) {
            # Preserve the supplied PNG bytes in the source directory.
            [IO.File]::Copy($inputPath, $sourceAtlasPath, $true)
        } else {
            $atlas.Save($sourceAtlasPath, [Drawing.Imaging.ImageFormat]::Png)
        }
    }
    # Generated sheets can have uneven margins. Locate the actual clear gutters
    # so no neighboring icon fragments are included in a cell.
    $rowCuts = [ToolbarMask]::Cuts($atlas, $true)
    $columnCuts = [ToolbarMask]::Cuts($atlas, $false)
    foreach ($size in @(32, 48, 64, 128)) {
        $strip = [Drawing.Bitmap]::new(64 * $size, $size, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
        $graphics = [Drawing.Graphics]::FromImage($strip)
        $graphics.CompositingMode = [Drawing.Drawing2D.CompositingMode]::SourceCopy
        $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $graphics.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        try {
            for ($index = 0; $index -lt 64; $index++) {
                $column = $index % 8
                $row = [Math]::Floor($index / 8)
                $left = $columnCuts[$column]
                $top = $rowCuts[$row]
                $right = $columnCuts[$column + 1]
                $bottom = $rowCuts[$row + 1]
                $tile = $atlas.Clone([Drawing.Rectangle]::new($left, $top, $right - $left, $bottom - $top), [Drawing.Imaging.PixelFormat]::Format32bppArgb)
                try {
                    $bounds = [ToolbarMask]::Apply($tile)
                    $factor = ($size * 0.84) / [Math]::Max($bounds.Width, $bounds.Height)
                    $width = [int][Math]::Round($bounds.Width * $factor)
                    $height = [int][Math]::Round($bounds.Height * $factor)
                    $destination = [Drawing.Rectangle]::new($index * $size + [int](($size - $width) / 2), [int](($size - $height) / 2), $width, $height)
                    $graphics.DrawImage($tile, $destination, $bounds.X, $bounds.Y, $bounds.Width, $bounds.Height, [Drawing.GraphicsUnit]::Pixel)
                } finally { $tile.Dispose() }
            }
        } finally { $graphics.Dispose() }
        try {
            $strip.Save((Join-Path $exportPath "toolbar-$size.png"), [Drawing.Imaging.ImageFormat]::Png)
            # Legacy BMP consumers must use white (RGB 255,255,255) as their mask.
            $bmp = [Drawing.Bitmap]::new(64 * $size, $size, [Drawing.Imaging.PixelFormat]::Format24bppRgb)
            $bmpGraphics = [Drawing.Graphics]::FromImage($bmp)
            try {
                $bmpGraphics.Clear([Drawing.Color]::White)
                $bmpGraphics.DrawImageUnscaled($strip, 0, 0)
                $bmp.Save((Join-Path $exportPath "toolbar-$size.bmp"), [Drawing.Imaging.ImageFormat]::Bmp)
            } finally { $bmpGraphics.Dispose(); $bmp.Dispose() }
            if ($size -eq 128) {
                for ($index = 0; $index -lt 64; $index++) {
                    $tile = $strip.Clone([Drawing.Rectangle]::new($index * $size, 0, $size, $size), [Drawing.Imaging.PixelFormat]::Format32bppArgb)
                    try { $tile.Save((Join-Path $exportIconPath ('{0:D2}.png' -f $index)), [Drawing.Imaging.ImageFormat]::Png) }
                    finally { $tile.Dispose() }
                }
            }
        } finally { $strip.Dispose() }
    }
    $preview = [Drawing.Bitmap]::new(768, 768)
    $previewGraphics = [Drawing.Graphics]::FromImage($preview)
    try {
        $previewGraphics.Clear([Drawing.Color]::FromArgb(244, 247, 250))
        for ($index = 0; $index -lt 64; $index++) {
            $tile = [Drawing.Bitmap]::new((Join-Path $exportIconPath ('{0:D2}.png' -f $index)))
            try {
                $previewGraphics.DrawImage($tile, [Drawing.Rectangle]::new(($index % 8) * 96, [Math]::Floor($index / 8) * 96, 96, 96))
            } finally { $tile.Dispose() }
        }
        $preview.Save((Join-Path $exportPath 'preview.png'), [Drawing.Imaging.ImageFormat]::Png)
    } finally { $previewGraphics.Dispose(); $preview.Dispose() }
    $master = [Drawing.Bitmap]::new(1024, 1024, [Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $masterGraphics = [Drawing.Graphics]::FromImage($master)
    try {
        for ($index = 0; $index -lt 64; $index++) {
            $tile = [Drawing.Bitmap]::new((Join-Path $exportIconPath ('{0:D2}.png' -f $index)))
            try { $masterGraphics.DrawImageUnscaled($tile, ($index % 8) * 128, [Math]::Floor($index / 8) * 128) }
            finally { $tile.Dispose() }
        }
        $master.Save((Join-Path $iconPath 'toolbar.png'), [Drawing.Imaging.ImageFormat]::Png)
    } finally { $masterGraphics.Dispose(); $master.Dispose() }
} finally { $atlas.Dispose() }
Write-Output "Saved the main atlas to $(Join-Path $iconPath 'toolbar.png'); exported 64 icons and 32/48/64/128 px strips to $exportPath"
