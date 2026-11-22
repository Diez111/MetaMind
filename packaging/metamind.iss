; Windows installer, built by .github/workflows/release.yml:
;   iscc /DAppVersion=1.2.3 /DSourceDir=<folder with metamind.exe> /DOutputName=<name> packaging\metamind.iss
; Installs per user (no administrator prompt) so `metamind update` can replace the program file.

#ifndef AppVersion
  #error Pass /DAppVersion=<version>
#endif
#ifndef SourceDir
  #error Pass /DSourceDir=<folder with metamind.exe>
#endif
#ifndef OutputName
  #define OutputName "metamind-setup"
#endif

[Setup]
AppId={{6F1D2C3A-8B4E-4C5F-9A7D-52B3E1F0C9A4}
AppName=MetaMind
AppVersion={#AppVersion}
AppPublisher=MetaMind
DefaultDirName={localappdata}\Programs\MetaMind
DefaultGroupName=MetaMind
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
UninstallDisplayIcon={app}\metamind.exe

[Tasks]
Name: desktopicon; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#SourceDir}\metamind.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\MetaMind"; Filename: "{app}\metamind.exe"
Name: "{userdesktop}\MetaMind"; Filename: "{app}\metamind.exe"; Tasks: desktopicon

[Registry]
; metamind:// links.
Root: HKCU; Subkey: "Software\Classes\metamind"; ValueType: string; ValueData: "URL:MetaMind link"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\metamind"; ValueName: "URL Protocol"; ValueType: string; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\metamind\DefaultIcon"; ValueType: string; ValueData: """{app}\metamind.exe"",0"
Root: HKCU; Subkey: "Software\Classes\metamind\shell\open\command"; ValueType: string; ValueData: """{app}\metamind.exe"" ""%1"""
; Markdown files: offered under "Open with", without taking over the user's default program.
Root: HKCU; Subkey: "Software\Classes\MetaMind.md"; ValueType: string; ValueData: "Markdown note"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\MetaMind.md\DefaultIcon"; ValueType: string; ValueData: """{app}\metamind.exe"",0"
Root: HKCU; Subkey: "Software\Classes\MetaMind.md\shell\open\command"; ValueType: string; ValueData: """{app}\metamind.exe"" ""%1"""
Root: HKCU; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueName: "MetaMind.md"; ValueType: none; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\.markdown\OpenWithProgids"; ValueName: "MetaMind.md"; ValueType: none; Flags: uninsdeletevalue
Root: HKCU; Subkey: "Software\Classes\Applications\metamind.exe\SupportedTypes"; ValueName: ".md"; ValueType: string; ValueData: ""; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\Applications\metamind.exe\shell\open\command"; ValueType: string; ValueData: """{app}\metamind.exe"" ""%1"""

[Run]
Filename: "{app}\metamind.exe"; Description: "Start MetaMind"; Flags: nowait postinstall skipifsilent
