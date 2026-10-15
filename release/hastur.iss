; Build with scripts/package-beta.ps1. PayloadRoot contains an explicit allowlist.
#ifndef PayloadRoot
  #error PayloadRoot is required
#endif
#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef Publisher
  #define Publisher "Hastur Retouch"
#endif
#ifndef ModelFilesInclude
  #error ModelFilesInclude is required
#endif

[Setup]
AppId={{C86EB2BC-1F1C-4485-B78C-14FF3CF7278B}
AppName=Hastur Retouch Beta
AppVersion={#AppVersion}
AppVerName=Hastur Retouch {#AppVersion}
AppPublisher={#Publisher}
AppSupportURL=mailto:suboptimald1@gmail.com
DefaultDirName={localappdata}\Programs\Hastur Retouch Beta
DefaultGroupName=Hastur Retouch Beta
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
LicenseFile={#PayloadRoot}\LICENSE
InfoBeforeFile=beta-info.txt
OutputDir={#ArtifactsRoot}
OutputBaseFilename=Hastur-Retouch-{#AppVersion}-windows-x64-setup
SetupIconFile=..\assets\branding\hastur.ico
UninstallDisplayIcon={app}\hastur-retouch.exe
UninstallDisplayName=Hastur Retouch {#AppVersion}
VersionInfoVersion={#NumericVersion}
VersionInfoProductName=Hastur Retouch
VersionInfoProductVersion={#NumericVersion}
VersionInfoProductTextVersion={#AppVersion}
WizardStyle=modern
#ifdef FastCompression
Compression=lzma2/fast
LZMANumBlockThreads=2
#else
; A single solid dictionary can reuse unchanged weight bytes across ONNX and
; Burn packs. Compression uses the separate 64-bit helper; extraction needs
; roughly 256 MB of dictionary memory and keeps every installed file exact.
Compression=lzma2/max
LZMADictionarySize=262144
LZMAUseSeparateProcess=yes
LZMANumBlockThreads=1
#endif
SolidCompression=yes
CloseApplications=yes
RestartApplications=no
#ifdef SignRelease
SignTool=hastur_release
SignedUninstaller=yes
#else
SignedUninstaller=no
#endif

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#PayloadRoot}\*"; DestDir: "{app}"; Excludes: "*.onnx,*.bpk"; Flags: ignoreversion recursesubdirs createallsubdirs
#include ModelFilesInclude

[Icons]
Name: "{group}\Hastur Retouch Beta"; Filename: "{app}\hastur-retouch.exe"; WorkingDir: "{app}"
Name: "{group}\Beta quick start"; Filename: "{app}\Quick-start.html"
Name: "{group}\Uninstall Hastur Retouch Beta"; Filename: "{uninstallexe}"
Name: "{autodesktop}\Hastur Retouch Beta"; Filename: "{app}\hastur-retouch.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\hastur-retouch.exe"; Description: "Open Hastur Retouch"; Flags: nowait postinstall skipifsilent unchecked

; Uninstall removes only installer-owned program files. AppData workspaces,
; preferences, caches, imported photos and exported images are preserved.
