; The Windows installer for tt-git-gui, built with Inno Setup 6 by
; package.ps1, which passes:
;
;   AppVersion  the release's version (0.9.0)
;   FileVersion AppVersion without a pre-release suffix (0.9.0)
;   StageDir    the folder with tt-git-gui.exe, tt-git.exe and README.md
;   OutputBase  the installer's file name without .exe
;   Arch        x64compatible or arm64
;
; It installs per user without asking for admin rights (an admin can pick
; all users in the first dialog), adds a Start menu entry, offers a desktop
; shortcut and an "Open in tt-git" entry on folders in Explorer, and
; uninstalls from Settings > Apps. tt-git.exe goes in bin, where the GUI
; finds it for plugins, and that folder goes on PATH unless the "path" task
; is unticked, so the terminal app works too. Only tt-git.exe is in bin, so
; nothing else lands on PATH.

#ifndef AppVersion
  #error AppVersion is required
#endif

[Setup]
; Never change AppId: it's how an upgrade finds the installed copy.
AppId={{8C46E8A7-87BB-4C0F-ACE5-D817FAE44C10}
AppName=tt-git
AppVersion={#AppVersion}
AppVerName=tt-git {#AppVersion}
AppPublisher=ThirtyTech
AppPublisherURL=https://github.com/ThirtyTech/tt-git
AppSupportURL=https://github.com/ThirtyTech/tt-git/issues
AppUpdatesURL=https://github.com/ThirtyTech/tt-git/releases
VersionInfoVersion={#FileVersion}
DefaultDirName={autopf}\tt-git
DefaultGroupName=tt-git
DisableProgramGroupPage=yes
DisableWelcomePage=no
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed={#Arch}
ArchitecturesInstallIn64BitMode={#Arch}
OutputDir=.
OutputBaseFilename={#OutputBase}
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\tt-git-gui.exe
UninstallDisplayName=tt-git
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; The modern wizard's image sizes at 100% DPI: sidebar.bmp is sidebar.svg
; at 202x386 (24-bit), small.bmp is ../../assets/icon.svg at 58x58 (32-bit,
; straight alpha). Render them again after changing either SVG.
WizardImageAlphaFormat=defined
WizardImageFile=sidebar.bmp
WizardSmallImageFile=small.bmp
; Tells running programs (Explorer, so new terminals) that PATH changed.
ChangesEnvironment=yes

[Tasks]
Name: desktopicon; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: explorer; Description: "Add ""Open in tt-git"" to the menu of folders in Explorer"; GroupDescription: "{cm:AdditionalIcons}"
Name: path; Description: "Add tt-git to PATH, to run it in a terminal"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#StageDir}\tt-git-gui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\tt-git.exe"; DestDir: "{app}\bin"; Flags: ignoreversion
Source: "{#StageDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion

; Earlier releases put tt-git.exe next to tt-git-gui.exe.
[InstallDelete]
Type: files; Name: "{app}\tt-git.exe"

; Backups the terminal app's install script leaves in bin, when it had
; installed there before the desktop app.
[UninstallDelete]
Type: files; Name: "{app}\bin\tt-git.exe.old*"

; AppUserModelID is the app's own (src/app_id.rs): notifications from an
; installed copy need it on the Start menu entry.
[Icons]
Name: "{autoprograms}\tt-git"; Filename: "{app}\tt-git-gui.exe"; AppUserModelID: "ThirtyTech.tt-git"
Name: "{autodesktop}\tt-git"; Filename: "{app}\tt-git-gui.exe"; AppUserModelID: "ThirtyTech.tt-git"; Tasks: desktopicon

; Explorer's "Open in tt-git", on a folder and on a folder's background.
; HKA is HKCU for a per-user install and HKLM for all users.
[Registry]
Root: HKA; Subkey: "Software\Classes\Directory\shell\tt-git"; ValueType: string; ValueName: ""; ValueData: "Open in tt-git"; Flags: uninsdeletekey; Tasks: explorer
Root: HKA; Subkey: "Software\Classes\Directory\shell\tt-git"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\tt-git-gui.exe"",0"; Tasks: explorer
Root: HKA; Subkey: "Software\Classes\Directory\shell\tt-git\command"; ValueType: string; ValueName: ""; ValueData: """{app}\tt-git-gui.exe"" ""%1"""; Tasks: explorer
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\tt-git"; ValueType: string; ValueName: ""; ValueData: "Open in tt-git"; Flags: uninsdeletekey; Tasks: explorer
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\tt-git"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\tt-git-gui.exe"",0"; Tasks: explorer
Root: HKA; Subkey: "Software\Classes\Directory\Background\shell\tt-git\command"; ValueType: string; ValueName: ""; ValueData: """{app}\tt-git-gui.exe"" ""%V"""; Tasks: explorer
; The app registers its AppUserModelID here: its icon (IconUri, a copy in
; the state folder) at every start (src/notify/icon.rs), and its name the
; first time it shows a notification (GPUI's set_app_identity).
; Uninstalling removes it.
Root: HKCU; Subkey: "Software\Classes\AppUserModelId\ThirtyTech.tt-git"; Flags: uninsdeletekey dontcreatekey

[Run]
Filename: "{app}\tt-git-gui.exe"; Description: "{cm:LaunchProgram,tt-git}"; Flags: nowait postinstall skipifsilent

; The PATH entry for bin: set after every install (added, or removed when
; the "path" task was unticked on an upgrade) and removed on uninstall.
; The user's PATH for an install for you alone, the system's for all users.
[Code]
function EnvRoot: Integer;
begin
  if IsAdminInstallMode then
    Result := HKEY_LOCAL_MACHINE
  else
    Result := HKEY_CURRENT_USER;
end;

function EnvKey: String;
begin
  if IsAdminInstallMode then
    Result := 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
  else
    Result := 'Environment';
end;

// Path without its entries for Dir (in any case, with or without a
// trailing backslash). Every other entry stays as it was, empty ones too.
function WithoutDir(Path, Dir: String): String;
var
  Rest, Entry: String;
  I: Integer;
  First: Boolean;
begin
  Result := '';
  First := True;
  Rest := Path + ';';
  while Rest <> '' do
  begin
    I := Pos(';', Rest);
    Entry := Copy(Rest, 1, I - 1);
    Delete(Rest, 1, I);
    if CompareText(RemoveBackslashUnlessRoot(Trim(Entry)), RemoveBackslashUnlessRoot(Dir)) <> 0 then
    begin
      if not First then
        Result := Result + ';';
      Result := Result + Entry;
      First := False;
    end;
  end;
end;

procedure SetBinOnPath(Add: Boolean);
var
  Path, Bin, Others: String;
begin
  Bin := ExpandConstant('{app}\bin');
  if not RegQueryStringValue(EnvRoot, EnvKey, 'Path', Path) then
    Path := '';
  Others := WithoutDir(Path, Bin);
  // Already there, as the terminal app's install script adds the same
  // folder for an install for you alone: leave it where it is.
  if Add and (Others <> Path) then
    Exit;
  if Add then
  begin
    if Others <> '' then
      Others := Others + ';';
    Others := Others + Bin;
  end;
  if Others <> Path then
    if not RegWriteExpandStringValue(EnvRoot, EnvKey, 'Path', Others) then
      Log('Couldn''t update PATH');
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    SetBinOnPath(WizardIsTaskSelected('path'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    SetBinOnPath(False);
end;
