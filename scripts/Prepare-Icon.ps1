$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies @([System.Drawing.Bitmap].Assembly.Location, [System.Drawing.Rectangle].Assembly.Location) -TypeDefinition @'
using System;
using System.Drawing;
public static class QuickIconBounds {
    public static Rectangle Find(Bitmap image) {
        int left = image.Width, top = image.Height, right = -1, bottom = -1;
        for (int y = 0; y < image.Height; y++) {
            for (int x = 0; x < image.Width; x++) {
                if (image.GetPixel(x, y).A < 16) continue;
                if (x < left) left = x;
                if (y < top) top = y;
                if (x > right) right = x;
                if (y > bottom) bottom = y;
            }
        }
        if (right < left) throw new InvalidOperationException("Icon has no visible pixels");
        return Rectangle.FromLTRB(left, top, right + 1, bottom + 1);
    }
}
'@
$assets = Join-Path $PSScriptRoot '..\assets'

function Render-Icon([System.Drawing.Bitmap]$Source, [System.Drawing.Rectangle]$Bounds, [int]$Size, [int]$Padding) {
    $bitmap = [System.Drawing.Bitmap]::new($Size, $Size)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $scale = ($Size - 2 * $Padding) / [Math]::Max($Bounds.Width, $Bounds.Height)
        $width = [int][Math]::Round($Bounds.Width * $scale)
        $height = [int][Math]::Round($Bounds.Height * $scale)
        $target = [System.Drawing.Rectangle]::new([int](($Size - $width) / 2), [int](($Size - $height) / 2), $width, $height)
        $graphics.DrawImage($Source, $target, $Bounds, [System.Drawing.GraphicsUnit]::Pixel)
        return $bitmap
    }
    finally { $graphics.Dispose() }
}

function Export-Icon([string]$Name, [int[]]$Sizes, [switch]$Tray) {
    $source = [System.Drawing.Bitmap]::new((Join-Path $assets "$Name.png"))
    try {
        $bounds = [QuickIconBounds]::Find($source)
        $images = @()
        foreach ($size in $Sizes) {
            $padding = if ($Tray) { 1 } else { [Math]::Max(1, [int][Math]::Round($size * 0.04)) }
            $bitmap = Render-Icon $source $bounds $size $padding
            $stream = [System.IO.MemoryStream]::new()
            try {
                $bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
                $images += ,$stream.ToArray()
            }
            finally { $bitmap.Dispose(); $stream.Dispose() }
        }
        $writer = [System.IO.BinaryWriter]::new([System.IO.File]::Create((Join-Path $assets "$Name.ico")))
        try {
            $writer.Write([uint16]0)
            $writer.Write([uint16]1)
            $writer.Write([uint16]$Sizes.Count)
            $offset = 6 + 16 * $Sizes.Count
            for ($index = 0; $index -lt $Sizes.Count; $index++) {
                $dimension = $Sizes[$index] % 256
                $writer.Write([byte]$dimension)
                $writer.Write([byte]$dimension)
                $writer.Write([uint16]0)
                $writer.Write([uint16]1)
                $writer.Write([uint16]32)
                $writer.Write([uint32]$images[$index].Length)
                $writer.Write([uint32]$offset)
                $offset += $images[$index].Length
            }
            foreach ($bytes in $images) { $writer.Write([byte[]]$bytes) }
        }
        finally { $writer.Dispose() }
        if (!$Tray) {
            $display = Render-Icon $source $bounds 256 8
            try { $display.Save((Join-Path $assets 'AppIconDisplay.png'), [System.Drawing.Imaging.ImageFormat]::Png) }
            finally { $display.Dispose() }
        }
        [pscustomobject]@{ Name = $Name; SourceSize = @($source.Width, $source.Height); VisibleBounds = $bounds.ToString(); Sizes = $Sizes } | ConvertTo-Json -Compress
    }
    finally { $source.Dispose() }
}

Export-Icon 'AppIcon' @(16, 20, 24, 32, 40, 48, 64, 96, 128, 256)
Export-Icon 'TrayIcon' @(16, 20, 24, 32) -Tray
