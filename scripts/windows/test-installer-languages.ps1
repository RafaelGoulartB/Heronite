<#
.SYNOPSIS
  Checks installer language coverage, format tokens and upgrade/uninstall
  safeguards without compiling or running Setup. No external test modules.
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

$failures = 0
function Check([bool]$Ok, [string]$What) {
    if ($Ok) { Write-Host "ok    $What" } else { Write-Host "FAIL  $What"; $script:failures++ }
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
foreach ($required in @('Setup', 'Languages', 'CustomMessages', 'Icons', 'Code')) {
    if (-not $sections.ContainsKey($required)) { throw "Missing [$required]" }
}

$expectedLanguages = [ordered]@{
    english = 'compiler:Default.isl'
    brazilianportuguese = 'compiler:Languages\BrazilianPortuguese.isl'
    spanish = 'compiler:Languages\Spanish.isl'
    german = 'compiler:Languages\German.isl'
}
$languages = @{}
foreach ($line in $sections.Languages) {
    if ($line -notmatch '^Name: "([^"]+)"; MessagesFile: "([^"]+)"$') {
        Check $false "valid language declaration: $line"
        continue
    }
    $name, $file = $Matches[1], $Matches[2]
    Check (-not $languages.ContainsKey($name)) "language $name is unique"
    $languages[$name] = $file
}
Check ((($languages.Keys | Sort-Object) -join '|') -ceq (($expectedLanguages.Keys | Sort-Object) -join '|')) 'exactly English, Brazilian Portuguese, Spanish and German'
Check ($sections.Languages[0] -cmatch '^Name: "english";') 'English is the fallback language'
foreach ($name in $expectedLanguages.Keys) {
    Check ($languages[$name] -ceq $expectedLanguages[$name]) "$name uses the bundled Inno translation"
}

$messages = @{}
foreach ($name in $expectedLanguages.Keys) { $messages[$name] = @{} }
foreach ($line in $sections.CustomMessages) {
    if ($line -notmatch '^([a-z]+)\.([A-Za-z][A-Za-z0-9]*)=(.*)$') {
        Check $false "qualified custom message: $line"
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

$english = @{
    DeleteUserData = 'Also delete your Heronite library, settings and cache?%n%n%1%n%2%n%nGames you installed are not in these folders and stay where they are. Choose No to keep everything for a later install.'
    PortableFolder = 'This folder holds a portable copy of Heronite (portable.txt). Choose another folder, or delete portable.txt to turn that copy into an installed one.'
    CloseRunning = 'Heronite is running. Close it and continue uninstalling?'
    ShortcutComment = 'Every game you own, in one launcher'
}
foreach ($key in $english.Keys) {
    Check ($messages.english[$key] -ceq $english[$key]) "English $key remains unchanged"
}
foreach ($language in $expectedLanguages.Keys) {
    $translated = $messages[$language]
    Check ((($translated.Keys | Sort-Object) -join '|') -ceq (($messages.english.Keys | Sort-Object) -join '|')) "$language custom message keys match English"
    foreach ($key in $messages.english.Keys) {
        $value = $translated[$key]
        Check ((Tokens $value) -ceq (Tokens $messages.english[$key])) "$language.$key preserves all format tokens"
        if ($language -ne 'english') {
            Check ($value -cne $messages.english[$key]) "$language.$key is translated"
        }
        foreach ($literal in @('Heronite', 'portable.txt')) {
            $count = [regex]::Matches($messages.english[$key], [regex]::Escape($literal)).Count
            Check ([regex]::Matches([string]$value, [regex]::Escape($literal)).Count -eq $count) "$language.$key preserves $literal"
        }
    }
}

# Every custom reference must resolve. These three messages come from Inno's
# bundled .isl files, rather than Heronite's [CustomMessages].
$builtins = @('CreateDesktopIcon', 'AdditionalIcons', 'LaunchProgram')
$references = @([regex]::Matches($source, "\{cm:([A-Za-z][A-Za-z0-9]*)|CustomMessage\('([^']+)'\)") |
    ForEach-Object { if ($_.Groups[1].Success) { $_.Groups[1].Value } else { $_.Groups[2].Value } })
foreach ($key in $references) {
    Check ($messages.english.ContainsKey($key) -or $key -cin $builtins) "custom reference $key resolves"
}
foreach ($key in $messages.english.Keys) {
    Check ($key -cin $references) "custom message $key is used"
}
foreach ($line in $sections.Icons) {
    Check ($line -cmatch 'Comment: "\{cm:ShortcutComment\}"') 'shortcut comment is localized'
}
$setup = $sections.Setup -join "`n"
Check ($setup -cmatch '(?m)^AppId=\{\{104041C7-B50B-4E70-8560-A6CFF660777E\}$') 'AppId is stable'
Check ($setup -cmatch '(?m)^UsePreviousLanguage=yes$') 'updates retain the selected language'
Check ($setup -cmatch '(?m)^ShowLanguageDialog=yes$') 'interactive setup offers language selection'
$code = $sections.Code -join "`n"
foreach ($dialog in [regex]::Matches($code, '(?:Suppressible)?MsgBox\([^\r\n]+')) {
    Check ($dialog.Value -cmatch '^(?:Suppressible)?MsgBox\((?:FmtMessage\()?CustomMessage\(') 'custom dialog text is localized'
}
Check ($code -cmatch "Result := CustomMessage\('PortableFolder'\);") 'portable-folder warning is localized'
Check ($code -cmatch "FmtMessage\(CustomMessage\('DeleteUserData'\), \[Roaming, Local\]\)") 'delete-data prompt substitutes both actual folders'
Check ($code -cmatch 'MB_YESNO or MB_DEFBUTTON2\) = IDYES') 'uninstall defaults to keeping user data'
Check ($code -cmatch 'if \(CurUninstallStep <> usPostUninstall\) or UninstallSilent\(\) then\s+exit;') 'silent uninstall never deletes user data'
Check ($code -cmatch "Result := \(not WizardSilent\(\)\) or \(ExpandConstant\('\{param:RELAUNCH\|0\}'\) = '1'\);") 'silent update only relaunches when requested'
Check ($code -cmatch "StrToIntDef\(ExpandConstant\('\{param:WAITPID\|0\}'\), 0\)") 'updater WAITPID parameter remains supported'

if ($failures -gt 0) { throw "$failures installer language check(s) failed" }
Write-Host 'all installer language checks passed'
