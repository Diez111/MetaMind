; Windows installer, built by .github/workflows/release.yml:
;   iscc /DAppVersion=1.2.3 /DSourceDir=<folder with rustsidian.exe> /DOutputName=<name> packaging\rustsidian.iss
; Installs per user (no administrator prompt) so `rustsidian update` can replace the program file.

#ifndef AppVersion
  #error Pass /DAppVersion=<version>
#endif
#ifndef SourceDir
  #error Pass /DSourceDir=<folder with rustsidian.exe>
#endif
#ifndef OutputName
  #define OutputName "rustsidian-setup"
#endif

[Setup]
AppId={{6F1D2C3A-8B4E-4C5F-9A7D-52B3E1F0C9A4}
AppName=Rustsidian
AppVersion={#AppVersion}
AppPublisher=Rustsidian
DefaultDirName={localappdata}\Programs\Rustsidian
DefaultGroupName=Rustsidian
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesAssociations=yes
OutputDir=.
OutputBaseFilename={#OutputName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\rustsidian.exe

[Tasks]
Name: desktopicon; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#SourceDir}\rustsidian.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\Rustsidian"; Filename: "{app}\rustsidian.exe"
Name: "{userdesktop}\Rustsidian"; Filename: "{app}\rustsidian.exe"; Tasks: desktopicon

[Registry]
; rustsidian:// links.
Root: HKCU; Subkey: "Software\Classes\rustsidian"; ValueType: string; ValueData: "URL:Rustsidian link"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\rustsidian"; ValueName: "URL Protocol"; ValueType: string; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\rustsidian\DefaultIcon"; ValueType: string; ValueData: """{app}\rustsidian.exe"",0"
Root: HKCU; Subkey: "Software\Classes\rustsidian\shell\open\command"; ValueType: string; ValueData: """{app}\rustsidian.exe"" ""%1"""
; Markdown files: offered under "Open with", without taking over the user's default program.
Root: HKCU; Subkey: "Software\Classes\Rustsidian.md"; ValueType: string; ValueData: "Markdown note"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Rustsidian.md\DefaultIcon"; ValueType: string; ValueData: """{app}\rustsidian.exe"",0"
Root: HKCU; Subkey: "Software\Classes\Rustsidian.md\shell\open\command"; ValueType: string; ValueData: """{app}\rustsidian.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueName: "Rustsidian.md"; ValueType: none; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.markdown\OpenWithProgids"; ValueName: "Rustsidian.md"; ValueType: none; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\Applications\rustsidian.exe\SupportedTypes"; ValueName: ".md"; ValueType: string; ValueData: ""; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Applications\rustsidian.exe\shell\open\command"; ValueType: string; ValueData: """{app}\rustsidian.exe"" ""%1"""

[Run]
Filename: "{app}\rustsidian.exe"; Description: "Start Rustsidian"; Flags: nowait postinstall skipifsilent
