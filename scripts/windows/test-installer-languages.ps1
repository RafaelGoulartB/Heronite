<#
.SYNOPSIS
  Checks the installer's translations without compiling or running Setup:
  every language of [Languages] uses a translation bundled with Inno Setup,
  has every custom message English has, with the same format tokens, and
  every text the user sees goes through a custom message. Prints only what
  fails. No external test modules.

  What Setup does (install, update, uninstall, the language it keeps) is
  checked by smoke.ps1 on the built setup, not here.
.PARAMETER Installer
  Installer source to check (defaults to packaging\windows\heronite.iss).
.EXAMPLE
  powershell -NoProfile -File scripts\windows\test-installer-languages.ps1
#>
param(
    [string]$Installer = (Join-Path $PSScriptRoot '..\..\packaging\windows\heronite.iss')
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$checks = 0
$failures = 0
function Check([bool]$Ok, [string]$What) {
    $script:checks++
    if (-not $Ok) { Write-Host "FAIL  $What"; $script:failures++ }
}

# Compare occurrences, not just distinct tokens: losing one line break or
# duplicating one of the paths must fail too. Inno permits reordered arguments.
function Tokens([string]$Text) {
    return (@([regex]::Matches($Text, '%(?:[1-9][0-9]*|n|%)|\{[^{}]+\}') |
        ForEach-Object { $_.Value } | Sort-Object -CaseSensitive) -join '|')
}

$source = Get-Content -LiteralPath $Installer -Raw -Encoding UTF8
$sections = @{}
$section = ''
foreach ($line in ($source -split '\r?\n')) {
    if ($line -match '^\s*\[([^\]]+)\]\s*$') {
        $section = $Matches[1]
        Check (-not $sections.ContainsKey($section)) "section $section is unique"
        $sections[$section] = @()
    } elseif ($line.Trim() -and $line -notmatch '^\s*;' -and $section) {
        $sections[$section] += $line
    }
}
foreach ($required in @('Languages', 'CustomMessages', 'Code')) {
    if (-not $sections.ContainsKey($required)) { throw "Missing [$required]" }
}

# Languages: English first (Setup falls back to it) and only Inno's own
# translations, so no translation file has to be kept in this repository.
$languages = [ordered]@{}
foreach ($line in $sections.Languages) {
    if ($line -notmatch '^Name: "([^"]+)"; MessagesFile: "([^"]+)"$') {
        Check $false "valid language declaration: $line"
        continue
    }
    $name, $file = $Matches[1], $Matches[2]
    Check (-not $languages.Contains($name)) "language $name is unique"
    Check ($file -cmatch '^compiler:') "$name uses a translation bundled with Inno Setup ($file)"
    $languages[$name] = $file
}
$first = @($languages.Keys)[0]
Check ($first -ceq 'english' -and $languages['english'] -ceq 'compiler:Default.isl') 'English (compiler:Default.isl) is the first language'

$messages = @{}
foreach ($name in $languages.Keys) { $messages[$name] = @{} }
foreach ($line in $sections.CustomMessages) {
    if ($line -notmatch '^([a-z]+)\.([A-Za-z][A-Za-z0-9]*)=(.*)$') {
        Check $false "custom message qualified by its language: $line"
        continue
    }
    $language, $key, $value = $Matches[1], $Matches[2], $Matches[3]
    if (-not $messages.ContainsKey($language)) {
        Check $false "custom message $key has a declared language ($language)"
        continue
    }
    Check (-not $messages[$language].ContainsKey($key)) "$language.$key is unique"
    Check (-not [string]::IsNullOrWhiteSpace($value)) "$language.$key is not empty"
    $messages[$language][$key] = $value
}
if (-not $messages.ContainsKey('english')) { throw 'No English custom messages' }
$english = $messages.english

foreach ($language in $languages.Keys) {
    $translated = $messages[$language]
    Check ((($translated.Keys | Sort-Object) -join '|') -ceq (($english.Keys | Sort-Object) -join '|')) "$language has the same custom messages as English"
    foreach ($key in $english.Keys) {
        if (-not $translated.ContainsKey($key)) { continue }
        $value = $translated[$key]
        Check ((Tokens $value) -ceq (Tokens $english[$key])) "$language.$key keeps the format tokens of English"
        if ($language -ne 'english') {
            Check ($value -cne $english[$key]) "$language.$key is translated"
        }
        # Names the user has to recognise (the app, the portable marker).
        foreach ($literal in @('Heronite', 'portable.txt')) {
            $count = [regex]::Matches($english[$key], [regex]::Escape($literal)).Count
            Check ([regex]::Matches($value, [regex]::Escape($literal)).Count -eq $count) "$language.$key keeps $literal as English has it"
        }
    }
}

# Every reference resolves and every message is used. These come from Inno's
# bundled .isl files rather than [CustomMessages].
$builtins = @('CreateDesktopIcon', 'AdditionalIcons', 'LaunchProgram')
$references = @([regex]::Matches($source, "\{cm:([A-Za-z][A-Za-z0-9]*)|CustomMessage\('([^']+)'\)") |
    ForEach-Object { if ($_.Groups[1].Success) { $_.Groups[1].Value } else { $_.Groups[2].Value } })
foreach ($key in $references) {
    Check ($english.ContainsKey($key) -or $key -cin $builtins) "custom message $key exists"
}
foreach ($key in $english.Keys) {
    Check ($key -cin $references) "custom message $key is used"
}

# The texts the user sees: shortcut comments, task and run descriptions, and
# the dialogs of [Code].
foreach ($name in @('Icons', 'Tasks', 'Run')) {
    if (-not $sections.ContainsKey($name)) { continue }
    foreach ($line in $sections[$name]) {
        foreach ($field in [regex]::Matches($line, '\b(Comment|Description|GroupDescription): "([^"]*)"')) {
            Check ($field.Groups[2].Value -cmatch '^\{cm:') "[$name] $($field.Groups[1].Value) is a custom message: $line"
        }
    }
}
$code = $sections.Code -join "`n"
foreach ($dialog in [regex]::Matches($code, '(?:Suppressible)?MsgBox\([^\r\n]+')) {
    Check ($dialog.Value -cmatch '^(?:Suppressible)?MsgBox\((?:FmtMessage\()?CustomMessage\(') "dialog text is a custom message: $($dialog.Value)"
}

if ($failures -gt 0) { throw "$failures of $checks installer language check(s) failed" }
Write-Host "installer languages: $checks checks passed ($($languages.Keys -join ', '))"
