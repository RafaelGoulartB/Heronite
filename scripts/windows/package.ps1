<#
.SYNOPSIS
  Builds Heronite's Windows release files in dist\:
    Heronite-Portable-x86_64.zip  one Heronite\ folder: heronite.exe (the cargo build's heronite-rs.exe),
                                  helpers\heronite-epic-login.exe (the
                                  stores' login window, out of the way of
                                  the program the user starts),
                                  portable.txt (keeps the data in that folder)
                                  and LICENSE
    Heronite-Setup-x86_64.exe     the per-user installer (Inno Setup 6,
                                  packaging\windows\heronite.iss)
  The minisign signatures for the in-app updater are made afterwards by
  scripts/sign-release.sh, with the Linux files.

  Authenticode: when a certificate is configured (WINDOWS_SIGN_COMMAND or
  WINDOWS_SIGN_PFX, see sign.ps1), heronite-rs.exe, heronite-epic-login.exe,
  the setup and its uninstaller are signed; otherwise they are not, and
  SmartScreen warns.

  Needs: Rust (MSVC), Inno Setup 6 (ISCC.exe: -Iscc, the ISCC variable,
  PATH, or its usual install folders). Windows PowerShell 5.1 or pwsh.

.PARAMETER Version
  The release version (X.Y.Z). Default: the workspace version in Cargo.toml.
.PARAMETER NoBuild
  Package the heronite-rs.exe and heronite-epic-login.exe already built in
  target\release.
.PARAMETER Iscc
  Path to ISCC.exe.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\windows\package.ps1
#>
param(
    [string]$Version,
    [switch]$NoBuild,
    [string]$Iscc
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Fail([string]$Message) {
    Write-Error "package.ps1: $Message" -ErrorAction Continue
    exit 1
}

$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$Packaging = Join-Path $Root 'packaging\windows'
$Dist = Join-Path $Root 'dist'
$ZipName = 'Heronite-Portable-x86_64.zip'
$SetupName = 'Heronite-Setup-x86_64.exe'

# ISCC.exe of Inno Setup 6.
function Find-Iscc {
    $candidates = @()
    if ($Iscc) { $candidates += $Iscc }
    if ($env:ISCC) { $candidates += $env:ISCC }
    $cmd = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($cmd) { $candidates += $cmd.Source }
    # Where its installer put it (machine or per-user install).
    foreach ($hive in @('HKLM:', 'HKCU:')) {
        foreach ($view in @('SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall', 'SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall')) {
            $key = "$hive\$view\Inno Setup 6_is1"
            $loc = (Get-ItemProperty -Path $key -Name InstallLocation -ErrorAction SilentlyContinue)
            if ($loc) { $candidates += (Join-Path $loc.InstallLocation 'ISCC.exe') }
        }
    }
    foreach ($dir in @(${env:ProgramFiles(x86)}, $env:ProgramFiles, (Join-Path $env:LOCALAPPDATA 'Programs'))) {
        if ($dir) { $candidates += (Join-Path $dir 'Inno Setup 6\ISCC.exe') }
    }
    foreach ($c in $candidates) {
        if ($c -and (Test-Path -LiteralPath $c -PathType Leaf)) { return $c }
    }
    Fail 'ISCC.exe (Inno Setup 6) not found: install it (winget install JRSoftware.InnoSetup), put it on PATH, or pass -Iscc'
}

# True when a code signing certificate is configured (see sign.ps1).
function Test-Signing {
    return [bool]($env:WINDOWS_SIGN_COMMAND -or $env:WINDOWS_SIGN_PFX)
}

if (-not $Version) {
    $line = Select-String -Path (Join-Path $Root 'Cargo.toml') -Pattern '^version = "(.*)"$' | Select-Object -First 1
    if (-not $line) { Fail 'no version = "..." in Cargo.toml' }
    $Version = $line.Matches[0].Groups[1].Value
}
if ($Version -notmatch '^\d+\.\d+\.\d+$') { Fail "version '$Version' is not X.Y.Z" }
Write-Host "Heronite $Version"

$IsccExe = Find-Iscc
Write-Host "ISCC: $IsccExe"

Push-Location $Root
try {
    if (-not $NoBuild) {
        & cargo build --release -p heronite -p heronite-epic-login --locked
        if ($LASTEXITCODE -ne 0) { Fail "cargo build failed ($LASTEXITCODE)" }
    }
    $meta = & cargo metadata --format-version 1 --no-deps --locked | ConvertFrom-Json
    if ($LASTEXITCODE -ne 0) { Fail "cargo metadata failed ($LASTEXITCODE)" }
} finally {
    Pop-Location
}
$BinDir = Join-Path $meta.target_directory 'release'
$Exe = Join-Path $BinDir 'heronite-rs.exe'
$LoginExe = Join-Path $BinDir 'heronite-epic-login.exe'
foreach ($f in @($Exe, $LoginExe)) {
    if (-not (Test-Path -LiteralPath $f)) { Fail "$f not found (build it, or drop -NoBuild)" }
}

$Signing = Test-Signing
$SignScript = Join-Path $PSScriptRoot 'sign.ps1'
if ($Signing) {
    foreach ($f in @($Exe, $LoginExe)) {
        & powershell.exe -NoProfile -ExecutionPolicy Bypass -File $SignScript $f
        if ($LASTEXITCODE -ne 0) { Fail "signing $(Split-Path -Leaf $f) failed" }
    }
} else {
    Write-Host 'No code signing certificate configured: the files are not Authenticode-signed.'
}

New-Item -ItemType Directory -Force $Dist | Out-Null
foreach ($name in @($ZipName, $SetupName)) {
    foreach ($f in @($name, "$name.minisig")) {
        $p = Join-Path $Dist $f
        if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Force }
    }
}

# Portable zip: exactly these files, under Heronite/ (forward slashes: the
# zip format's separator; Compress-Archive in PowerShell 5.1 writes
# backslashes). Never data, config or cache.
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$ZipPath = Join-Path $Dist $ZipName
$entries = [ordered]@{
    'Heronite/heronite.exe'            = $Exe
    'Heronite/helpers/heronite-epic-login.exe' = $LoginExe
    'Heronite/portable.txt'            = (Join-Path $Packaging 'portable.txt')
    'Heronite/LICENSE'                 = (Join-Path $Root 'LICENSE')
}
$zip = [System.IO.Compression.ZipFile]::Open($ZipPath, [System.IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($e in $entries.GetEnumerator()) {
        [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile($zip, $e.Value, $e.Key, [System.IO.Compression.CompressionLevel]::Optimal) | Out-Null
    }
} finally {
    $zip.Dispose()
}
Write-Host "wrote $ZipPath"

# Installer.
$isccArgs = @('/Q', "/DAppVersion=$Version", "/DBinDir=$BinDir", "/DOutputDir=$Dist")
if ($Signing) {
    # Inno Setup runs this for the setup and the uninstaller; $f is the file,
    # $q a quote. The certificate settings reach sign.ps1 through the
    # environment, so no secret is on a command line.
    $isccArgs += '/DSign'
    $isccArgs += "/Sheronite=powershell.exe -NoProfile -ExecutionPolicy Bypass -File `$q$SignScript`$q `$f"
}
$isccArgs += (Join-Path $Packaging 'heronite.iss')
& $IsccExe @isccArgs
if ($LASTEXITCODE -ne 0) { Fail "ISCC failed ($LASTEXITCODE)" }
$SetupPath = Join-Path $Dist $SetupName
if (-not (Test-Path -LiteralPath $SetupPath)) { Fail "ISCC did not write $SetupPath" }
Write-Host "wrote $SetupPath"

foreach ($p in @($ZipPath, $SetupPath)) {
    $item = Get-Item -LiteralPath $p
    $hash = (Get-FileHash -LiteralPath $p -Algorithm SHA256).Hash.ToLower()
    Write-Host ('{0}  {1,12:N0} bytes  sha256 {2}' -f $item.Name, $item.Length, $hash)
}
