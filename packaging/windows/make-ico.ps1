# Makes the installer's icon (an .ico) from the app icon's PNGs in
# crates/desktop/assets/icon/, as crates/desktop/build.rs does for the
# executable: one PNG-compressed entry per size, each PNG as it is.
#
#   pwsh packaging/windows/make-ico.ps1 -Out target/adelite.ico
param(
    # Where to write the .ico.
    [Parameter(Mandatory)][string]$Out
)
$ErrorActionPreference = "Stop"
$icons = Join-Path $PSScriptRoot "..\..\crates\desktop\assets\icon"

# The pre-rendered sizes are named by their side in pixels; macos.png is the Mac's.
$pngs = @(Get-ChildItem $icons -Filter *.png | Where-Object { $_.BaseName -match '^\d+$' } |
    Sort-Object { [int]$_.BaseName })
if ($pngs.Count -eq 0) { throw "no icon PNGs in $icons" }

$images = @(foreach ($png in $pngs) {
    $bytes = [IO.File]::ReadAllBytes($png.FullName)
    if ($bytes.Length -lt 24 -or $bytes[1] -ne 0x50 -or $bytes[2] -ne 0x4E -or $bytes[3] -ne 0x47) {
        throw "$($png.Name) is not a PNG"
    }
    # The size from the PNG's own header (IHDR: big-endian width, then height).
    $width = ([int]$bytes[18] -shl 8) -bor $bytes[19]
    $height = ([int]$bytes[22] -shl 8) -bor $bytes[23]
    if ($bytes[16] -ne 0 -or $bytes[17] -ne 0 -or $width -ne $height -or $width -gt 256) {
        throw "$($png.Name) is not a square of at most 256 pixels"
    }
    @{ Side = $width; Bytes = $bytes }
})

$path = $ExecutionContext.SessionState.Path.GetUnresolvedProviderPathFromPSPath($Out)
New-Item -ItemType Directory -Force (Split-Path $path) | Out-Null
$writer = [IO.BinaryWriter]::new([IO.File]::Create($path))
try {
    # ICONDIR: reserved, type 1 (an icon), the number of images.
    $writer.Write([uint16]0)
    $writer.Write([uint16]1)
    $writer.Write([uint16]$images.Count)
    # ICONDIRENTRY per image, its PNG following all of them.
    $offset = 6 + 16 * $images.Count
    foreach ($image in $images) {
        # 256 is written as 0.
        $side = [byte]($image.Side % 256)
        $writer.Write($side)
        $writer.Write($side)
        $writer.Write([byte]0)      # no palette
        $writer.Write([byte]0)      # reserved
        $writer.Write([uint16]1)    # planes
        $writer.Write([uint16]32)   # bits per pixel
        $writer.Write([uint32]$image.Bytes.Length)
        $writer.Write([uint32]$offset)
        $offset += $image.Bytes.Length
    }
    foreach ($image in $images) { $writer.Write([byte[]]$image.Bytes) }
} finally {
    $writer.Dispose()
}
Write-Host "== $path ($($images.Count) sizes: $(($images | ForEach-Object Side) -join ', '))"
