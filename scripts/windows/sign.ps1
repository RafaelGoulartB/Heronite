<#
.SYNOPSIS
  Authenticode-signs one file (heronite-rs.exe, heronite-epic-login.exe,
  the setup, its uninstaller) with the certificate configured in the
  environment. package.ps1 calls it, and hands it to Inno Setup as its sign
  tool, only when one is configured (see Test-Signing in package.ps1);
  without a certificate nothing is signed and Windows SmartScreen warns
  about the downloads.

  Configure one of:
    WINDOWS_SIGN_COMMAND   a full command line; $f is replaced by the quoted
                           file name. For Azure Trusted Signing, for example:
                           signtool.exe sign /v /fd SHA256 /tr http://timestamp.acs.microsoft.com /td SHA256
                             /dlib <path>\Azure.CodeSigning.Dlib.dll /dmdf <path>\metadata.json $f
    WINDOWS_SIGN_PFX       path to a .pfx certificate, with
    WINDOWS_SIGN_PASSWORD  its password, and optionally
    WINDOWS_SIGN_TIMESTAMP an RFC 3161 timestamp server
                           (default http://timestamp.digicert.com)

.EXAMPLE
  scripts\windows\sign.ps1 target\release\heronite-rs.exe
#>
param(
    [Parameter(Mandatory = $true)][string]$File
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Fail([string]$Message) {
    Write-Error "sign.ps1: $Message" -ErrorAction Continue
    exit 1
}

# The newest signtool.exe of the installed Windows SDKs, or the one on PATH.
function Find-SignTool {
    $cmd = Get-Command signtool.exe -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }
    $kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    if (Test-Path $kits) {
        $found = Get-ChildItem $kits -Directory |
            Where-Object { Test-Path (Join-Path $_.FullName 'x64\signtool.exe') } |
            Sort-Object { [version]($_.Name -replace '[^0-9.]', '') } -Descending |
            Select-Object -First 1
        if ($found) { return (Join-Path $found.FullName 'x64\signtool.exe') }
    }
    Fail 'signtool.exe not found (install the Windows SDK or put it on PATH)'
}

# Runs a command line through cmd.exe exactly as written (no PowerShell
# re-quoting of the arguments).
function Invoke-CommandLine([string]$CommandLine) {
    $psi = New-Object System.Diagnostics.ProcessStartInfo
    $psi.FileName = Join-Path $env:SystemRoot 'System32\cmd.exe'
    $psi.Arguments = '/d /s /c "' + $CommandLine + '"'
    $psi.UseShellExecute = $false
    $p = [System.Diagnostics.Process]::Start($psi)
    $p.WaitForExit()
    return $p.ExitCode
}

if (-not (Test-Path -LiteralPath $File -PathType Leaf)) { Fail "no such file: $File" }
$File = (Resolve-Path -LiteralPath $File).Path

if ($env:WINDOWS_SIGN_COMMAND) {
    $code = Invoke-CommandLine ($env:WINDOWS_SIGN_COMMAND.Replace('$f', '"' + $File + '"'))
    if ($code -ne 0) { Fail "WINDOWS_SIGN_COMMAND failed ($code) for $File" }
} elseif ($env:WINDOWS_SIGN_PFX) {
    if (-not (Test-Path -LiteralPath $env:WINDOWS_SIGN_PFX)) { Fail "WINDOWS_SIGN_PFX not found: $env:WINDOWS_SIGN_PFX" }
    $timestamp = 'http://timestamp.digicert.com'
    if ($env:WINDOWS_SIGN_TIMESTAMP) { $timestamp = $env:WINDOWS_SIGN_TIMESTAMP }
    $signtool = Find-SignTool
    $signArgs = @('sign', '/fd', 'SHA256', '/f', $env:WINDOWS_SIGN_PFX)
    if ($env:WINDOWS_SIGN_PASSWORD) { $signArgs += @('/p', $env:WINDOWS_SIGN_PASSWORD) }
    $signArgs += @('/tr', $timestamp, '/td', 'SHA256', '/d', 'Heronite', '/du', 'https://github.com/RafaelGoulartB/Heronite', $File)
    & $signtool @signArgs | Out-Host
    if ($LASTEXITCODE -ne 0) { Fail "signtool sign failed ($LASTEXITCODE) for $File" }
} else {
    Fail 'no certificate configured (WINDOWS_SIGN_COMMAND or WINDOWS_SIGN_PFX)'
}

$sig = Get-AuthenticodeSignature -LiteralPath $File
if (-not $sig.SignerCertificate -or @('NotSigned', 'HashMismatch', 'NotSupportedFileFormat', 'Incompatible') -contains "$($sig.Status)") {
    Fail "signature of $File is $($sig.Status): $($sig.StatusMessage)"
}
if ($sig.Status -ne 'Valid') {
    # A test certificate: signed, but not by a certificate Windows trusts.
    Write-Warning "signature of $File is $($sig.Status): $($sig.StatusMessage)"
}
Write-Host "signed $File ($($sig.SignerCertificate.Subject))"
