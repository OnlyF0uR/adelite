# Puts Microsoft's Visual C++ runtime DLLs beside Adelite in the staging folder,
# and checks that nothing there needs one that is not beside it.
#
# adelite.exe links the runtime in (.cargo/config.toml), but TDLib's DLLs, and
# the OpenSSL and zlib ones it needs, are built against the DLL runtime
# (VCRUNTIME140.dll, VCRUNTIME140_1.dll, MSVCP140.dll). A fresh Windows has
# none of those: without them beside the exe, Adelite does not even start.
# Microsoft allows them to be shipped this way, in the app's own folder; the
# Universal CRT (api-ms-win-crt-*) is part of Windows 10 and is not copied.
#
#   ./packaging/windows/add-runtime.ps1 -Stage stage
param(
    # The folder holding adelite.exe and its DLLs.
    [Parameter(Mandatory)][string]$Stage
)
$ErrorActionPreference = "Stop"

$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
$vs = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
if (-not $vs) { throw "no Visual Studio with the C++ tools found" }

# The newest runtime Visual Studio has: never older than what built the DLLs.
$crt = Get-ChildItem (Join-Path $vs "VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT") -Directory |
    Sort-Object FullName | Select-Object -Last 1
if (-not $crt) { throw "no Visual C++ runtime found in $vs" }
$dumpbin = Get-ChildItem (Join-Path $vs "VC\Tools\MSVC\*\bin\Hostx64\x64\dumpbin.exe") |
    Sort-Object FullName | Select-Object -Last 1
if (-not $dumpbin) { throw "no dumpbin.exe found in $vs" }

# The runtime DLLs whatever is staged imports, by name.
function Needed([string]$file) {
    & $dumpbin.FullName /nologo /dependents $file |
        ForEach-Object { $_.Trim() } |
        Where-Object { $_ -match '^(vcruntime|msvcp|concrt|vccorlib)\d.*\.dll$' }
}

$needed = Get-ChildItem $Stage -Include *.exe, *.dll -File -Recurse |
    ForEach-Object { Needed $_.FullName } | Sort-Object -Unique
foreach ($name in $needed) {
    $from = Join-Path $crt.FullName $name
    if (-not (Test-Path $from)) { throw "$name is needed but not in $($crt.FullName)" }
    Copy-Item $from $Stage -Force
    Write-Host "== $name from $($crt.FullName)"
}

# What was copied may need more of the runtime in turn (msvcp140 needs vcruntime140).
$missing = Get-ChildItem $Stage -Include *.exe, *.dll -File -Recurse |
    ForEach-Object { Needed $_.FullName } | Sort-Object -Unique |
    Where-Object { -not (Test-Path (Join-Path $Stage $_)) }
if ($missing) { throw "still missing beside the app: $($missing -join ', ')" }
$exe = Join-Path $Stage "adelite.exe"
if ((Test-Path $exe) -and (Needed $exe)) {
    throw "adelite.exe needs the runtime DLLs: it was built without .cargo/config.toml's crt-static"
}
