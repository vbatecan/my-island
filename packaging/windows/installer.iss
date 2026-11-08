; Per-user desktop installer. The MSIX remains the notification-capable package.
; Build through scripts/build-installer.ps1; no tool download or OS changes occur
; during compilation. Keep AppId stable so future versions upgrade this install.
#ifndef AppVersion
  #error AppVersion must be supplied by build-installer.ps1
#endif
#ifndef NumericVersion
  #error NumericVersion must be supplied by build-installer.ps1
#endif
#ifndef SourceExecutable
  #error SourceExecutable must point to the static-CRT x64 release executable
#endif
#ifndef RepoRoot
  #error RepoRoot must be supplied by build-installer.ps1
#endif
#ifndef OutputDirectory
  #error OutputDirectory must be supplied by build-installer.ps1
#endif

[Setup]
AppId={{FA56F811-8F7A-4F1A-AD38-94DCB332EAF8}
AppName=Still Island
AppVersion={#AppVersion}
AppVerName=Still Island {#AppVersion}
AppPublisher=Still Island contributors
VersionInfoVersion={#NumericVersion}
DefaultDirName={localappdata}\Programs\StillIsland
DefaultGroupName=Still Island
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
MinVersion=10.0.22621
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
AppMutex=Local\StillIsland.Singleton
CloseApplications=yes
CloseApplicationsFilter=still-island.exe
RestartApplications=no
UninstallDisplayName=Still Island
UninstallDisplayIcon={app}\still-island.exe
LicenseFile={#RepoRoot}\LICENSE
OutputDir={#OutputDirectory}
OutputBaseFilename=StillIsland-{#AppVersion}-windows-x64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupLogging=yes
ChangesAssociations=no
ChangesEnvironment=no
AllowNoIcons=yes

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#SourceExecutable}"; DestDir: "{app}"; DestName: "still-island.exe"; Flags: ignoreversion
Source: "{#RepoRoot}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#RepoRoot}\README.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\Still Island"; Filename: "{app}\still-island.exe"; WorkingDir: "{app}"
Name: "{userdesktop}\Still Island"; Filename: "{app}\still-island.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\still-island.exe"; Description: "Launch Still Island"; WorkingDir: "{app}"; Flags: postinstall nowait skipifsilent unchecked

; Preferences live outside {app}. Startup remains opt-in in the application.
; Uninstall removes only the Run value pointing to this exact installation.
[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  StartupCommand: String;
begin
  if CurUninstallStep = usUninstall then
  begin
    if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run',
      'StillIsland', StartupCommand) then
    begin
      if CompareText(StartupCommand, '"' + ExpandConstant('{app}\still-island.exe') + '"') = 0 then
        RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'StillIsland');
    end;
  end;
end;
