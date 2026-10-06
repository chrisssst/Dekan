#define MyAppName "Dekan"
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-manual"
#endif
#if Pos("-", MyAppVersion) > 0
  #define MyAppNumericVersion Copy(MyAppVersion, 1, Pos("-", MyAppVersion) - 1)
#else
  #define MyAppNumericVersion MyAppVersion
#endif
#define MyAppPublisher "Dekan"
#define MyAppDescription "League of Legends skin changer for Windows"
#define MyAppCopyright "Dekan build. Original Bullet copyright (c) 2026 Isllan Toso. MIT License."
#define MyAppURL "https://github.com/chrisssst/Dekan"
#define MyPublisherURL "https://github.com/chrisssst/Dekan"
#define MyAppExeName "dekan.exe"

[Setup]
AppId={{A7231B49-6B20-4DAF-A3D0-8F5296B987C2}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyPublisherURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
AppCopyright={#MyAppCopyright}
AppComments={#MyAppDescription}
VersionInfoVersion={#MyAppNumericVersion}
VersionInfoTextVersion={#MyAppVersion}
VersionInfoProductName={#MyAppName}
VersionInfoProductVersion={#MyAppNumericVersion}
VersionInfoProductTextVersion={#MyAppVersion}
VersionInfoCompany={#MyAppPublisher}
VersionInfoDescription={#MyAppName} Setup - {#MyAppDescription}
VersionInfoCopyright={#MyAppCopyright}
VersionInfoOriginalFileName=Dekan-Setup-{#MyAppVersion}-x64.exe
UninstallDisplayName={#MyAppName} {#MyAppVersion}
DefaultDirName={commonpf}\{#MyAppName}
UsePreviousAppDir=no
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
OutputDir=..\dist\installer
OutputBaseFilename=Dekan-Setup-{#MyAppVersion}-x64
SetupIconFile=..\assets\dekan.ico
LicenseFile=..\LICENSE
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
AppMutex=Local\Dekan_SingleInstance_dekan
CloseApplications=yes
RestartApplications=no
UninstallDisplayIcon={app}\{#MyAppExeName}

[Languages]
Name: "turkish"; MessagesFile: "compiler:Languages\Turkish.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
Name: "autostart"; Description: "{cm:AutoStartProgram,{#MyAppName}}"; GroupDescription: "{cm:AutoStartProgramGroupDescription}"; Flags: unchecked

[InstallDelete]
Type: filesandordirs; Name: "{localappdata}\Programs\Dekan"
Type: files; Name: "{app}\tools\*.orig"
Type: files; Name: "{app}\tools\*.bak"
Type: filesandordirs; Name: "{localappdata}\Dekan\overlay"

[Dirs]
Name: "{app}\tools"
Name: "{localappdata}\Dekan\state"
Name: "{localappdata}\Dekan\library"

[Files]
Source: "..\dist\dekan.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\dekan.ico"; DestDir: "{app}\assets"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\dist\party.json"; DestDir: "{localappdata}\Dekan\state"; Flags: onlyifdoesntexist

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\assets\dekan.ico"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\assets\dekan.ico"; Tasks: desktopicon

[Registry]
Root: HKLM; Subkey: "SOFTWARE\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers"; ValueType: none; ValueName: "{app}\{#MyAppExeName}"; Flags: deletevalue uninsdeletevalue
Root: HKCU; Subkey: "Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers"; ValueType: none; ValueName: "{app}\{#MyAppExeName}"; Flags: deletevalue uninsdeletevalue
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Dekan"; ValueData: """{app}\{#MyAppExeName}"""; Flags: uninsdeletevalue; Tasks: autostart
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Dekan"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent runasoriginaluser

[UninstallDelete]
Type: filesandordirs; Name: "{localappdata}\Dekan\logs"
Type: filesandordirs; Name: "{localappdata}\Dekan\state"
Type: filesandordirs; Name: "{localappdata}\Dekan\webview2"
Type: filesandordirs; Name: "{localappdata}\Dekan\overlay"
Type: filesandordirs; Name: "{localappdata}\Dekan\mods"
Type: filesandordirs; Name: "{localappdata}\Dekan\tools"
Type: files; Name: "{app}\tools\ltk_patcher_host.exe"
Type: files; Name: "{app}\tools\ltk_patcher_dll.dll"
Type: dirifempty; Name: "{app}\tools"
Type: dirifempty; Name: "{app}\assets"
Type: dirifempty; Name: "{app}"

[CustomMessages]
turkish.DeleteUserContent=Dekan tarafından kaydedilen özel skinler ve modlar da kaldırılsın mı?%n%n%1%n%nSaklamak için "Hayır" seçeneğini seçin.
english.DeleteUserContent=Also remove the skins and custom mods saved by Dekan?%n%n%1%n%nChoose "No" to keep them.

[Code]
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir, Listing: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    DataDir := ExpandConstant('{localappdata}\Dekan');
    if DirExists(DataDir + '\library') or DirExists(DataDir + '\skins') or DirExists(DataDir + '\custom_mods') then
    begin
      Listing := DataDir + '\library' + #13#10 + DataDir + '\skins' + #13#10 + DataDir + '\custom_mods';
      if SuppressibleMsgBox(FmtMessage(CustomMessage('DeleteUserContent'), [Listing]), mbConfirmation, MB_YESNO, IDNO) = IDYES then
      begin
        DelTree(DataDir + '\library', True, True, True);
        DelTree(DataDir + '\skins', True, True, True);
        DelTree(DataDir + '\custom_mods', True, True, True);
      end;
    end;
    RemoveDir(DataDir);
  end;
end;

