#ifndef AppVersion
  #error AppVersion must be set
#endif
[Setup]
AppId={{F7DD70AA-7F1A-4D31-9E47-3E499FEC696A}
AppName=Lightmail
AppVersion={#AppVersion}
AppPublisher=Lightmail contributors
AppPublisherURL=https://github.com/baileyh8/lightmail
DefaultDirName={localappdata}\Programs\Lightmail
DefaultGroupName=Lightmail
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir=..\dist
OutputBaseFilename=Lightmail-v{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\Lightmail.exe
CloseApplications=yes
LicenseFile=..\LICENSE
[Files]
Source: "..\target\release\Lightmail.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\packaging\windows-readme.txt"; DestDir: "{app}"; DestName: "README.txt"; Flags: ignoreversion
[Icons]
Name: "{group}\Lightmail"; Filename: "{app}\Lightmail.exe"
[Run]
Filename: "{app}\Lightmail.exe"; Description: "Open Lightmail"; Flags: nowait postinstall skipifsilent
; Intentionally no UninstallDelete entries: preserve mail, drafts, and settings.
