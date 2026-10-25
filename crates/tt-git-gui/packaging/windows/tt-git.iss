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
; all users in the first dialog), adds a Start menu entry and uninstalls
; from Settings > Apps. tt-git.exe goes next to tt-git-gui.exe for plugins;
; the installer doesn't put it on PATH.

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

[Tasks]
Name: desktopicon; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#StageDir}\tt-git-gui.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\tt-git.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\tt-git"; Filename: "{app}\tt-git-gui.exe"
Name: "{autodesktop}\tt-git"; Filename: "{app}\tt-git-gui.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\tt-git-gui.exe"; Description: "{cm:LaunchProgram,tt-git}"; Flags: nowait postinstall skipifsilent
