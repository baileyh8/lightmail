# Convert the existing macOS artwork to a multi-resolution Windows icon.
# No new artwork or build-time Python/ImageMagick dependency is required.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
$root = Split-Path $PSScriptRoot -Parent
$bytes = [IO.File]::ReadAllBytes((Join-Path $root 'Resources/AppIcon.icns'))
$source = $null
for ($offset = 8; $offset -lt $bytes.Length;) {
    $length = ([int]$bytes[$offset + 4] -shl 24) -bor ([int]$bytes[$offset + 5] -shl 16) -bor ([int]$bytes[$offset + 6] -shl 8) -bor [int]$bytes[$offset + 7]
    if ($length -lt 8 -or $offset + $length -gt $bytes.Length) { throw 'Invalid ICNS resource' }
    if ([Text.Encoding]::ASCII.GetString($bytes, $offset, 4) -eq 'ic10') {
        $source = [IO.MemoryStream]::new($bytes, $offset + 8, $length - 8)
        break
    }
    $offset += $length
}
if (-not $source) { throw 'ICNS is missing its 1024px PNG' }
$art = [Drawing.Image]::FromStream($source)
$frames = @()
try {
    foreach ($size in @(16, 20, 24, 32, 40, 48, 64, 128, 256)) {
        $bitmap = [Drawing.Bitmap]::new($size, $size)
        $graphics = [Drawing.Graphics]::FromImage($bitmap)
        $png = [IO.MemoryStream]::new()
        try {
            $graphics.InterpolationMode = [Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
            $graphics.PixelOffsetMode = [Drawing.Drawing2D.PixelOffsetMode]::HighQuality
            $graphics.DrawImage($art, 0, 0, $size, $size)
            $bitmap.Save($png, [Drawing.Imaging.ImageFormat]::Png)
            $frames += [pscustomobject]@{ size = $size; bytes = $png.ToArray() }
        } finally { $png.Dispose(); $graphics.Dispose(); $bitmap.Dispose() }
    }
} finally { $art.Dispose(); $source.Dispose() }
$file = [IO.File]::Create((Join-Path $root 'Resources/AppIcon.ico'))
$writer = [IO.BinaryWriter]::new($file)
try {
    $writer.Write([uint16]0); $writer.Write([uint16]1); $writer.Write([uint16]$frames.Count)
    $offset = 6 + 16 * $frames.Count
    foreach ($frame in $frames) {
        $sizeByte = if ($frame.size -eq 256) { 0 } else { $frame.size }
        $writer.Write([byte]$sizeByte); $writer.Write([byte]$sizeByte)
        $writer.Write([byte]0); $writer.Write([byte]0)
        $writer.Write([uint16]1); $writer.Write([uint16]32)
        $writer.Write([uint32]$frame.bytes.Length); $writer.Write([uint32]$offset)
        $offset += $frame.bytes.Length
    }
    foreach ($frame in $frames) { $writer.Write([byte[]]$frame.bytes) }
} finally { $writer.Dispose() }
