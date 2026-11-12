# Builds TDLib's JSON interface (tdjson.dll) from Telegram's source, the way
# https://tdlib.github.io/td/build.html describes for Windows, and copies it
# with the DLLs it needs into vendor/tdlib/, where the app looks for it.
#
# Needs: git and Visual Studio 2022 with the C++ workload. Everything else
# (vcpkg, CMake, gperf, OpenSSL, zlib) is fetched into vendor/td/.
#
#   pwsh packaging/tdlib/build-windows.ps1
param(
    # The TDLib commit to build: pinned in COMMIT beside this script, so every
    # build, on every system, is the same library.
    [string]$Ref = (Get-Content (Join-Path $PSScriptRoot "COMMIT") -TotalCount 1).Trim(),
    # Compilers run at once. TDLib's largest files take over 2 GB each.
    [int]$Jobs = 8
)
$ErrorActionPreference = "Stop"
$root = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$td = Join-Path $root "vendor\td"
$out = Join-Path $root "vendor\tdlib"

function Run([string]$what, [scriptblock]$command) {
    Write-Host "== $what"
    & $command
    if ($LASTEXITCODE -ne 0) { throw "$what failed with exit code $LASTEXITCODE" }
}

if (-not (Test-Path (Join-Path $td ".git"))) {
    Run "clone TDLib" { git clone https://github.com/tdlib/td.git $td }
}
Run "check out $Ref" { git -C $td fetch origin; git -C $td checkout --detach $Ref }

$vcpkg = Join-Path $td "vcpkg"
if (-not (Test-Path (Join-Path $vcpkg ".git"))) {
    Run "clone vcpkg" { git clone https://github.com/microsoft/vcpkg.git $vcpkg }
}
if (-not (Test-Path (Join-Path $vcpkg "vcpkg.exe"))) {
    Run "bootstrap vcpkg" { & (Join-Path $vcpkg "bootstrap-vcpkg.bat") -disableMetrics }
}
Run "install gperf, OpenSSL and zlib" {
    & (Join-Path $vcpkg "vcpkg.exe") install gperf:x64-windows openssl:x64-windows zlib:x64-windows
}

# vcpkg fetched a CMake of its own to build those with: use it when none is installed.
$cmake = (Get-Command cmake -ErrorAction SilentlyContinue).Source
if (-not $cmake) {
    $cmake = Get-ChildItem (Join-Path $vcpkg "downloads\tools") -Recurse -Filter cmake.exe |
        Select-Object -First 1 -ExpandProperty FullName
}
if (-not $cmake) { throw "no cmake.exe found; install CMake and run this again" }

$build = Join-Path $td "build"
New-Item -ItemType Directory -Force $build | Out-Null
Run "configure TDLib" {
    & $cmake -S $td -B $build -A x64 `
        "-DCMAKE_TOOLCHAIN_FILE:FILEPATH=$vcpkg\scripts\buildsystems\vcpkg.cmake"
}
Run "build tdjson" { & $cmake --build $build --target tdjson --config Release --parallel $Jobs }

New-Item -ItemType Directory -Force $out | Out-Null
Copy-Item (Join-Path $build "Release\*.dll") $out -Force
Write-Host "== done: $out"
Get-ChildItem $out -Filter *.dll | Select-Object Name, Length
