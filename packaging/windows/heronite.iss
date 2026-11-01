; Heronite's Windows installer (Inno Setup 6): Heronite-Setup-x86_64.exe.
; Built by scripts/windows/package.ps1, which passes the version:
;   ISCC /DAppVersion=X.Y.Z [/DBinDir=...] [/DOutputDir=...] heronite.iss
;
; Per user, no administrator rights: %LOCALAPPDATA%\Programs\Heronite, a Start
; menu shortcut, an optional desktop shortcut and an uninstaller. An installed
; copy is marked by {app}\package-format ("windows-installer"); it never has a
; portable.txt (that would make it keep its data in its own folder).
;
; The app updates itself by running the new setup with
;   /VERYSILENT /SUPPRESSMSGBOXES /NORESTART /NOCANCEL /WAITPID=<pid> /RELAUNCH=1
; /WAITPID=<pid>  waits (60 s at most) for that process (the running app) to
;                 exit before any file is replaced.
; /RELAUNCH=1     starts the installed Heronite after a silent install;
;                 without it a silent install starts nothing.
; Interactive installs offer the usual "Launch Heronite" checkbox instead.
;
; Uninstalling removes the program, its shortcuts and the registry entries
; Heronite writes for itself (the "Heronite" value of the user's Run key and
; HKCU\Software\Classes\heronite). The library and settings
; (%APPDATA%\heronite, %LOCALAPPDATA%\heronite) stay, unless the user answers
; Yes when an interactive uninstall asks; a silent uninstall never deletes them.

#ifndef AppVersion
  #error Pass the version: ISCC /DAppVersion=X.Y.Z heronite.iss
#endif
#ifndef BinDir
  #define BinDir "..\..\target\release"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

#define AppExe "heronite-rs.exe"
#define AppUrl "https://github.com/RafaelGoulartB/Heronite"

[Setup]
; Never change the AppId: Windows and Setup find the installed copy by it.
AppId={{104041C7-B50B-4E70-8560-A6CFF660777E}
AppName=Heronite
AppVersion={#AppVersion}
AppVerName=Heronite {#AppVersion}
AppPublisher=Heronite
AppPublisherURL=https://heronite.app
AppSupportURL={#AppUrl}/issues
AppUpdatesURL={#AppUrl}/releases
AppCopyright=GPL-3.0-only
VersionInfoVersion={#AppVersion}
VersionInfoProductName=Heronite
VersionInfoDescription=Heronite Setup
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\Heronite
DisableDirPage=auto
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=Heronite-Setup-x86_64
SetupIconFile=heronite.ico
UninstallDisplayName=Heronite
UninstallDisplayIcon={app}\{#AppExe}
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
; A running Heronite is closed (Restart Manager) before its files are
; replaced; the "Launch Heronite" checkbox or /RELAUNCH=1 starts it again.
CloseApplications=yes
RestartApplications=no
; Every run writes "Setup Log <date> #<n>.txt" to the user's %TEMP%
; (/LOG="file" picks the name).
SetupLogging=yes
; Two setups at once (two update attempts) would fight over the files.
SetupMutex=HeroniteSetup
#ifdef Sign
; package.ps1 defines the "heronite" sign tool (ISCC /Sheronite=...) when a
; code signing certificate is configured; Setup and the uninstaller are then
; signed like heronite-rs.exe and heronite-epic-login.exe.
SignTool=heronite
SignedUninstaller=yes
#endif

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
english.DeleteUserData=Also delete your Heronite library, settings and cache?%n%n%1%n%2%n%nGames you installed are not in these folders and stay where they are. Choose No to keep everything for a later install.
english.PortableFolder=This folder holds a portable copy of Heronite (portable.txt). Choose another folder, or delete portable.txt to turn that copy into an installed one.
english.CloseRunning=Heronite is running. Close it and continue uninstalling?

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#BinDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
; The stores' login window, in a subfolder so heronite-rs.exe is the only
; program at the top (the app looks for it there).
Source: "{#BinDir}\heronite-epic-login.exe"; DestDir: "{app}\helpers"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "package-format"; DestDir: "{app}"; Flags: ignoreversion

[InstallDelete]
; Where older versions put the login window.
Type: files; Name: "{app}\heronite-epic-login.exe"

[Icons]
Name: "{autoprograms}\Heronite"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; Comment: "Every game you own, in one launcher"
Name: "{autodesktop}\Heronite"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; Comment: "Every game you own, in one launcher"; Tasks: desktopicon

[Registry]
; Written by the app itself (system/autostart.rs, system/protocol.rs), not by
; Setup: dontcreatekey only makes the uninstaller delete them.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "Heronite"; Flags: dontcreatekey uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\heronite"; Flags: dontcreatekey uninsdeletekey

[Run]
Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; Description: "{cm:LaunchProgram,Heronite}"; Flags: nowait postinstall; Check: ShouldLaunch

[Code]
const
  SYNCHRONIZE = $00100000;
  WAIT_OBJECT_0 = 0;
  WAITPID_TIMEOUT_MS = 60000;

function OpenProcess(DesiredAccess: DWORD; InheritHandle: BOOL; ProcessId: DWORD): THandle;
  external 'OpenProcess@kernel32.dll stdcall';
function WaitForSingleObject(Handle: THandle; Milliseconds: DWORD): DWORD;
  external 'WaitForSingleObject@kernel32.dll stdcall';
function CloseHandle(Handle: THandle): BOOL;
  external 'CloseHandle@kernel32.dll stdcall';

{ Waits up to TimeoutMs for process Pid to exit. True when it has exited (or
  was already gone). }
function WaitForProcess(Pid: DWORD; TimeoutMs: DWORD): Boolean;
var
  Handle: THandle;
begin
  Handle := OpenProcess(SYNCHRONIZE, False, Pid);
  if Handle = 0 then begin
    Log(Format('Process %d is not running', [Pid]));
    Result := True;
    exit;
  end;
  Result := WaitForSingleObject(Handle, TimeoutMs) = WAIT_OBJECT_0;
  CloseHandle(Handle);
end;

{ /WAITPID=<pid>: the app that started this update exits first, so its
  files can be replaced. After the timeout Setup goes on: Restart Manager
  (CloseApplications) closes it if it still holds the files. }
function InitializeSetup(): Boolean;
var
  Pid: Integer;
begin
  Result := True;
  Pid := StrToIntDef(ExpandConstant('{param:WAITPID|0}'), 0);
  if Pid > 0 then begin
    Log(Format('Waiting for process %d to exit', [Pid]));
    if WaitForProcess(Pid, WAITPID_TIMEOUT_MS) then
      Log(Format('Process %d has exited', [Pid]))
    else
      Log(Format('Process %d still runs after %d ms; going on', [Pid, WAITPID_TIMEOUT_MS]));
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if FileExists(ExpandConstant('{app}\portable.txt')) then
    Result := CustomMessage('PortableFolder');
end;

{ Interactive installs: the "Launch Heronite" checkbox. Silent installs:
  only with /RELAUNCH=1 (the app's own updates). }
function ShouldLaunch(): Boolean;
begin
  Result := (not WizardSilent()) or (ExpandConstant('{param:RELAUNCH|0}') = '1');
end;

{ The process ids of the Heronite started from this installation (other
  copies, like a portable one, are left alone). }
function RunningCopies(): TArrayOfInteger;
var
  Locator, Service, Procs, Proc: Variant;
  Exe, Path: String;
  I, N: Integer;
begin
  SetArrayLength(Result, 0);
  Exe := Lowercase(ExpandConstant('{app}\{#AppExe}'));
  try
    Locator := CreateOleObject('WbemScripting.SWbemLocator');
    Service := Locator.ConnectServer('.', 'root\CIMV2');
    Procs := Service.ExecQuery('SELECT ProcessId, ExecutablePath FROM Win32_Process WHERE Name = ''{#AppExe}''');
    N := 0;
    for I := 0 to Procs.Count - 1 do begin
      Proc := Procs.ItemIndex(I);
      if VarIsNull(Proc.ExecutablePath) then
        continue;
      Path := Proc.ExecutablePath;
      if Lowercase(Path) = Exe then begin
        SetArrayLength(Result, N + 1);
        Result[N] := Proc.ProcessId;
        N := N + 1;
      end;
    end;
  except
    Log('Listing running copies failed: ' + GetExceptionMessage);
  end;
end;

{ Asks each copy to quit like a plain taskkill does (WM_CLOSE: Heronite
  saves and exits, even from the tray), then ends the ones still running
  after 10 s. }
procedure CloseCopies(Pids: TArrayOfInteger);
var
  I, Code: Integer;
begin
  for I := 0 to GetArrayLength(Pids) - 1 do begin
    Log(Format('Asking Heronite (process %d) to quit', [Pids[I]]));
    Exec(ExpandConstant('{sys}\taskkill.exe'), Format('/PID %d', [Pids[I]]), '', SW_HIDE, ewWaitUntilTerminated, Code);
  end;
  for I := 0 to GetArrayLength(Pids) - 1 do
    if not WaitForProcess(Pids[I], 10000) then begin
      Log(Format('Process %d did not quit; ending it', [Pids[I]]));
      Exec(ExpandConstant('{sys}\taskkill.exe'), Format('/F /PID %d', [Pids[I]]), '', SW_HIDE, ewWaitUntilTerminated, Code);
      WaitForProcess(Pids[I], 5000);
    end;
end;

{ The uninstaller has no Restart Manager step: a running Heronite would keep
  its program file. }
function InitializeUninstall(): Boolean;
var
  Pids: TArrayOfInteger;
begin
  Result := True;
  Pids := RunningCopies();
  if GetArrayLength(Pids) = 0 then
    exit;
  if not UninstallSilent() then
    if MsgBox(CustomMessage('CloseRunning'), mbConfirmation, MB_OKCANCEL) <> IDOK then begin
      Result := False;
      exit;
    end;
  CloseCopies(Pids);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Roaming, Local: String;
begin
  if (CurUninstallStep <> usPostUninstall) or UninstallSilent() then
    exit;
  Roaming := ExpandConstant('{userappdata}\heronite');
  Local := ExpandConstant('{localappdata}\heronite');
  if not DirExists(Roaming) and not DirExists(Local) then
    exit;
  if MsgBox(FmtMessage(CustomMessage('DeleteUserData'), [Roaming, Local]), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then begin
    DelTree(Roaming, True, True, True);
    DelTree(Local, True, True, True);
  end;
end;
