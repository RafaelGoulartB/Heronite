<#
.SYNOPSIS
  Checks the Windows packages that package.ps1 wrote to dist\:
  - the portable zip holds exactly Heronite/heronite.exe,
    Heronite/helpers/heronite-epic-login.exe, Heronite/portable.txt and
    Heronite/LICENSE;
  - the login window of the unpacked zip starts (--smoke: a local page)
    and keeps WebView2's files in that copy's cache folder;
  - the setup installs silently into a temporary folder (files, the
    package-format marker, no portable.txt, the Start menu shortcut, the
    uninstall entry), installs again over it the way the app updates itself
    (/WAITPID: it waits for a process to exit first), and uninstalls
    silently, removing the program, the shortcut, the uninstall entry and the
    registry entries the app writes for itself.
  Nothing is started from the installed copy (no /RELAUNCH). Registry entries
  the app wrote before the test (the Run value, heronite:// links) are saved
  and put back. Refuses to run when Heronite is already installed with this
  setup, so a real installation is never touched.

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\windows\smoke.ps1
#>
param(
    [string]$Dist
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
if (-not $Dist) { $Dist = Join-Path $Root 'dist' }
$Zip = Join-Path $Dist 'Heronite-Portable-x86_64.zip'
$Setup = Join-Path $Dist 'Heronite-Setup-x86_64.exe'
# The AppId of packaging\windows\heronite.iss.
$UninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{104041C7-B50B-4E70-8560-A6CFF660777E}_is1'
$RunKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$ClassesKey = 'HKCU:\Software\Classes\heronite'
$Shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Heronite.lnk'

$failures = 0
function Check([bool]$Ok, [string]$What) {
    if ($Ok) { Write-Host "ok    $What" } else { Write-Host "FAIL  $What"; $script:failures++ }
}

function Wait-Until([scriptblock]$Condition, [int]$Seconds) {
    $deadline = (Get-Date).AddSeconds($Seconds)
    while ((Get-Date) -lt $deadline) {
        if (& $Condition) { return $true }
        Start-Sleep -Milliseconds 250
    }
    return [bool](& $Condition)
}

function Invoke-Setup([string[]]$Arguments) {
    $p = Start-Process -FilePath $Setup -ArgumentList $Arguments -PassThru -Wait
    return $p.ExitCode
}

foreach ($f in @($Zip, $Setup)) {
    if (-not (Test-Path -LiteralPath $f)) { Write-Error "smoke.ps1: $f not found (run package.ps1 first)" -ErrorAction Continue; exit 1 }
}
if (Test-Path $UninstallKey) {
    Write-Error 'smoke.ps1: Heronite is installed with this setup on this computer; uninstall it first (the test would replace it)' -ErrorAction Continue
    exit 1
}

# Portable zip.
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::OpenRead($Zip)
try {
    $names = @($archive.Entries | ForEach-Object { $_.FullName } | Sort-Object)
} finally {
    $archive.Dispose()
}
$expected = @('Heronite/heronite.exe', 'Heronite/helpers/heronite-epic-login.exe', 'Heronite/portable.txt', 'Heronite/LICENSE' | Sort-Object)
Check (($names -join '|') -eq ($expected -join '|')) "zip holds exactly $($expected -join ', ') (has: $($names -join ', '))"

$work = Join-Path ([IO.Path]::GetTempPath()) ('heronite-smoke-' + [guid]::NewGuid().ToString('N').Substring(0, 8))
New-Item -ItemType Directory -Force $work | Out-Null

# The login window of the unpacked portable copy starts: --smoke loads a
# local page (no network), prints SMOKE-READY and closes after about 15 s.
# WebView2 keeps its files in the copy's cache folder, not next to the
# program. Without the WebView2 runtime it must say so with its status 5.
$portable = Join-Path $work 'portable'
[System.IO.Compression.ZipFile]::ExtractToDirectory($Zip, $portable)
$copy = Join-Path $portable 'Heronite'
$login = Start-Process -FilePath (Join-Path $copy 'helpers\heronite-epic-login.exe') -ArgumentList '--smoke' -PassThru `
    -RedirectStandardOutput "$work\login.out" -RedirectStandardError "$work\login.err"
$null = $login.Handle
if (-not $login.WaitForExit(60000)) { Stop-Process -Id $login.Id -Force; $login.WaitForExit() }
$loginOut = (Get-Content -Raw "$work\login.out" -ErrorAction SilentlyContinue)
$loginErr = (Get-Content -Raw "$work\login.err" -ErrorAction SilentlyContinue)
if ($login.ExitCode -eq 5) {
    Write-Host "skip  no WebView2 runtime here: $loginErr"
} else {
    Check ($login.ExitCode -eq 0 -and "$loginOut" -match 'SMOKE-READY') "login window --smoke exits with 0 after SMOKE-READY (got $($login.ExitCode): $loginOut $loginErr)"
    Check (Test-Path (Join-Path $copy 'cache\login-webview\EBWebView')) 'its WebView2 files are in the copy''s cache folder'
    Check (-not (Test-Path (Join-Path $copy 'helpers\heronite-epic-login.exe.WebView2'))) 'none next to the program'
}

# Saves what the app registered before the test, to put it back afterwards.
$dir = Join-Path $work 'Heronite Test'
$runBefore = (Get-ItemProperty -Path $RunKey -Name Heronite -ErrorAction SilentlyContinue)
$classesBackup = Join-Path $work 'classes.reg'
$hadClasses = Test-Path $ClassesKey
if ($hadClasses) {
    & reg.exe export 'HKCU\Software\Classes\heronite' $classesBackup /y | Out-Null
    if ($LASTEXITCODE -ne 0) { Write-Error 'smoke.ps1: could not back up HKCU\Software\Classes\heronite' -ErrorAction Continue; exit 1 }
}

try {
    # Install.
    $code = Invoke-Setup @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/NOCANCEL', "/DIR=`"$dir`"", "/LOG=`"$work\install.log`"")
    Check ($code -eq 0) "silent install exits with 0 (got $code)"
    Check (Test-Path (Join-Path $dir 'heronite-rs.exe')) 'heronite-rs.exe installed'
    Check (Test-Path (Join-Path $dir 'helpers\heronite-epic-login.exe')) 'helpers\heronite-epic-login.exe (the login window) installed'
    Check (-not (Test-Path (Join-Path $dir 'heronite-epic-login.exe'))) 'heronite-rs.exe is the only program at the top'
    Check (Test-Path (Join-Path $dir 'LICENSE')) 'LICENSE installed'
    Check (Test-Path (Join-Path $dir 'unins000.exe')) 'uninstaller installed'
    Check (-not (Test-Path (Join-Path $dir 'portable.txt'))) 'no portable.txt in the installed copy'
    $marker = ''
    if (Test-Path (Join-Path $dir 'package-format')) { $marker = (Get-Content -Raw (Join-Path $dir 'package-format')).Trim() }
    Check ($marker -eq 'windows-installer') "package-format holds windows-installer (got '$marker')"
    Check (Test-Path $Shortcut) "Start menu shortcut $Shortcut"
    if (Test-Path $Shortcut) {
        $target = (New-Object -ComObject WScript.Shell).CreateShortcut($Shortcut).TargetPath
        Check ($target -eq (Join-Path $dir 'heronite-rs.exe')) "shortcut starts the installed program ($target)"
    }
    Check (Test-Path $UninstallKey) 'uninstall entry registered'
    if (Test-Path $UninstallKey) {
        $entry = Get-ItemProperty $UninstallKey
        Check ($entry.DisplayName -eq 'Heronite') "uninstall entry named Heronite ($($entry.DisplayName))"
        Write-Host "      version $($entry.DisplayVersion), $($entry.InstallLocation)"
    }
    Check ($null -eq (Get-Process heronite-rs -ErrorAction SilentlyContinue | Where-Object { $_.Path -like "$dir\*" })) 'a silent install without /RELAUNCH starts nothing'

    # Update over it, the way the app does: /WAITPID makes Setup wait for
    # the given process (here a ping of about 6 s) before replacing files.
    $ping = Start-Process -FilePath "$env:SystemRoot\System32\PING.EXE" -ArgumentList '-n', '7', '127.0.0.1' -WindowStyle Hidden -PassThru
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $code = Invoke-Setup @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/NOCANCEL', "/WAITPID=$($ping.Id)", "/DIR=`"$dir`"", "/LOG=`"$work\update.log`"")
    $watch.Stop()
    Check ($code -eq 0) "silent update exits with 0 (got $code)"
    Check ($ping.HasExited) 'the waited-for process had exited when Setup finished'
    Check ($watch.Elapsed.TotalSeconds -ge 4) ("Setup waited for it ({0:N1} s)" -f $watch.Elapsed.TotalSeconds)
    $log = Get-Content -Raw "$work\update.log"
    Check ($log -match "Process $($ping.Id) has exited") 'the setup log records the wait'

    # The values the app writes for itself, so uninstalling can be seen to
    # remove them.
    New-ItemProperty -Path $RunKey -Name Heronite -Value "`"$dir\heronite-rs.exe`" --autostart" -PropertyType String -Force | Out-Null
    New-Item -Path "$ClassesKey\shell\open\command" -Force | Out-Null

    # Uninstall. The uninstaller hands over to a copy of itself in %TEMP%,
    # so wait for its results rather than for the process.
    $uninstaller = Join-Path $dir 'unins000.exe'
    $p = Start-Process -FilePath $uninstaller -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$work\uninstall.log`"" -PassThru -Wait
    Check ($p.ExitCode -eq 0) "silent uninstall exits with 0 (got $($p.ExitCode))"
    Check (Wait-Until { -not (Test-Path $UninstallKey) } 60) 'uninstall entry removed'
    Check (Wait-Until { -not (Test-Path $dir) } 30) 'program folder removed'
    Check (-not (Test-Path $Shortcut)) 'Start menu shortcut removed'
    Check ($null -eq (Get-ItemProperty -Path $RunKey -Name Heronite -ErrorAction SilentlyContinue)) 'Run value Heronite removed'
    Check (-not (Test-Path $ClassesKey)) 'HKCU\Software\Classes\heronite removed'
} finally {
    # Put back what the app had registered before the test.
    if ($runBefore) {
        New-ItemProperty -Path $RunKey -Name Heronite -Value $runBefore.Heronite -PropertyType String -Force | Out-Null
    } else {
        Remove-ItemProperty -Path $RunKey -Name Heronite -ErrorAction SilentlyContinue
    }
    if ($hadClasses) {
        # (reg.exe reports success on stderr; it is not redirected, so
        # Windows PowerShell does not turn it into an error.)
        & reg.exe import $classesBackup
    } elseif (Test-Path $ClassesKey) {
        Remove-Item -Path $ClassesKey -Recurse -Force
    }
    Write-Host "logs in $work"
}

if ($failures -gt 0) {
    Write-Host "$failures check(s) failed"
    exit 1
}
Write-Host 'all checks passed'
