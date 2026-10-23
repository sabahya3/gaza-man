; Inno Setup script for the Gaza Man installer.
; Built by GitHub Actions:  iscc /DAppVersion=2.0.0 packaging\windows\gaza-man.iss
; Expects the program at dist\gaza-man.exe (relative to the repository root).

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{8F3C2B71-5D4E-4A9B-9C1F-6A7D2E0B4C55}
AppName=Gaza Man
AppVersion={#AppVersion}
AppVerName=Gaza Man {#AppVersion}
AppPublisher=Ahmed El Sabahy
AppPublisherURL=https://github.com/sabahya3/gaza-man
AppSupportURL=https://github.com/sabahya3/gaza-man/issues
DefaultDirName={autopf}\Gaza Man
DefaultGroupName=Gaza Man
DisableProgramGroupPage=yes
; Installs for the current user, without admin rights (they can still choose "all users").
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
LicenseFile=..\..\LICENSE
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\Gaza Man.exe
UninstallDisplayName=Gaza Man
OutputDir=..\..\dist
OutputBaseFilename=Gaza-Man-Setup-windows-x86_64
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "..\..\dist\gaza-man.exe"; DestDir: "{app}"; DestName: "Gaza Man.exe"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Gaza Man"; Filename: "{app}\Gaza Man.exe"
Name: "{autodesktop}\Gaza Man"; Filename: "{app}\Gaza Man.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\Gaza Man.exe"; Description: "{cm:LaunchProgram,Gaza Man}"; Flags: nowait postinstall skipifsilent
