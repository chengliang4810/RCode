; RCode for Windows — per-user installer (Inno Setup 6).
;
; Built by scripts/package-windows.ps1, which passes the version, the package
; architecture, and the staged portable directory:
;   ISCC.exe /DAppVersion=0.0.1-beta /DAppNumericVersion=0.0.1 /DArch=x86_64 /DPackageDir=<stage> /DOutputDir=<out> rcode.iss
;
; Installs into %LOCALAPPDATA%\Programs\RCode without elevation, like VS
; Code's user setup: the directory stays writable by its user, so the in-app
; updater (crates/update/src/windows.rs) can replace rcode.exe in place. The
; staged directory already carries rcode-update.json, which marks the install
; as update-managed. Re-running a newer installer upgrades in place; user data
; lives in %USERPROFILE%\.rcode and is never touched here.

#ifndef AppVersion
  #error AppVersion must be defined (/DAppVersion=x.y.z)
#endif
#ifndef AppNumericVersion
  #define AppNumericVersion AppVersion
#endif
#ifndef Arch
  #error Arch must be defined (/DArch=x86_64 or /DArch=aarch64)
#endif
#ifndef PackageDir
  #error PackageDir must be defined (/DPackageDir=<staged package directory>)
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif

#if Arch == "aarch64"
  #define ArchAllowed "arm64"
#else
  #define ArchAllowed "x64compatible"
#endif

[Setup]
; Never change AppId: it identifies the installation across upgrades, and
; crates/update/src/windows.rs refreshes DisplayVersion under this key after
; in-app updates.
AppId={{AD5DEC34-E254-467B-8F24-8127EBAF4DA6}
AppName=RCode
AppVersion={#AppVersion}
AppVerName=RCode {#AppVersion}
AppPublisher=RCode
AppPublisherURL=https://github.com/chengliang4810/RCode
AppSupportURL=https://github.com/chengliang4810/RCode/issues
AppUpdatesURL=https://github.com/chengliang4810/RCode/releases
VersionInfoVersion={#AppNumericVersion}
PrivilegesRequired=lowest
DefaultDirName={autopf}\RCode
DisableProgramGroupPage=yes
DisableDirPage=auto
DisableReadyPage=yes
ArchitecturesAllowed={#ArchAllowed}
ArchitecturesInstallIn64BitMode={#ArchAllowed}
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=rcode-{#AppVersion}-windows-{#Arch}-setup
SetupIconFile=rcode.ico
UninstallDisplayIcon={app}\rcode.exe
UninstallDisplayName=RCode
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
; A running RCode is closed through the Restart Manager before its files are
; replaced; the updated app starts again from the finish page.
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#PackageDir}\rcode.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PackageDir}\rcode-update.json"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PackageDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PackageDir}\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PackageDir}\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#PackageDir}\licenses\*"; DestDir: "{app}\licenses"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\RCode"; Filename: "{app}\rcode.exe"
Name: "{autodesktop}\RCode"; Filename: "{app}\rcode.exe"; Tasks: desktopicon

[Registry]
; rcode:// conversation links — the scheme macOS registers in Info.plist and
; Linux in rcode.desktop.
Root: HKCU; Subkey: "Software\Classes\rcode"; ValueType: string; ValueName: ""; ValueData: "URL:RCode"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\rcode"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\rcode\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\rcode.exe"",0"
Root: HKCU; Subkey: "Software\Classes\rcode\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\rcode.exe"" ""%1"""

[Run]
Filename: "{app}\rcode.exe"; Description: "{cm:LaunchProgram,RCode}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; Leftovers of in-app updates (crates/update/src/windows.rs).
Type: files; Name: "{app}\rcode.exe.old"
Type: files; Name: "{app}\.rcode-update-incoming.exe"
Type: filesandordirs; Name: "{app}\.rcode-update-*"
