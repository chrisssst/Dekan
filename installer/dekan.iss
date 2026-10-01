; Inno Setup script for Dekan — League of Legends skin changer for Windows
; Architecture: x64 native

#define MyAppName "Dekan"
; Injected by `cargo xtask installer` from Cargo.toml, so the installer can never claim a version
; the binary does not have. The fallback only applies when ISCC is run by hand.
#ifndef MyAppVersion
  #define MyAppVersion "0.0.0-manual"
#endif
; VersionInfoVersion only accepts numbers (1.0.0.0): a pre-release suffix (1.0.0-rc.1) is cut here
; and kept in the text versions.
#if Pos("-", MyAppVersion) > 0
  #define MyAppNumericVersion Copy(MyAppVersion, 1, Pos("-", MyAppVersion) - 1)
#else
  #define MyAppNumericVersion MyAppVersion
#endif
#define MyAppPublisher "Dekan"
#define MyAppDescription "League of Legends skin changer for Windows"
#define MyAppCopyright "Dekan build. Original Bullet copyright (c) 2026 Isllan Toso. MIT License."
#define MyAppExeName "dekan.exe"

[Setup]
AppId={{A7231B49-6B20-4DAF-A3D0-8F5296B987C2}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppVerName={#MyAppName} {#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppCopyright={#MyAppCopyright}
AppComments={#MyAppDescription}
; Properties > Details of the Setup exe.
VersionInfoVersion={#MyAppNumericVersion}
VersionInfoTextVersion={#MyAppVersion}
VersionInfoProductName={#MyAppName}
VersionInfoProductVersion={#MyAppNumericVersion}
VersionInfoProductTextVersion={#MyAppVersion}
VersionInfoCompany={#MyAppPublisher}
VersionInfoDescription={#MyAppName} Setup - {#MyAppDescription}
VersionInfoCopyright={#MyAppCopyright}
; Programs and Features entry.
UninstallDisplayName={#MyAppName} {#MyAppVersion}
; Program Files, not %LOCALAPPDATA%\Programs: nothing that touches the game may live in a folder a
; normal user can write to, and the injection tools live under {app}\tools. With
; PrivilegesRequired=lowest, {autopf} would send the whole install to the user-writable LocalAppData.
DefaultDirName={commonpf}\{#MyAppName}
; Never inherit an old per-user install path on upgrade.
UsePreviousAppDir=no
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
OutputDir=..\dist\installer
; Conventional installer name: product, purpose, version and arch, so a user with several downloads
; can tell them apart (e.g. Dekan-Setup-0.1.0-x64.exe).
OutputBaseFilename=Dekan-Setup-{#MyAppVersion}-x64
SetupIconFile=..\assets\dekan.ico
; Shown as the license page of the wizard; the same files are installed next to the program.
LicenseFile=..\LICENSE
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
; Dekan lives in the tray, so on an upgrade it is almost always running. This is the mutex the app
; holds for its whole life (dekan-platform::single_instance): Setup detects it up front and asks the
; user to close Dekan, instead of failing to replace a file in use or demanding a reboot.
AppMutex=Local\Dekan_SingleInstance_dekan
CloseApplications=yes
RestartApplications=no
UninstallDisplayIcon={app}\{#MyAppExeName}

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
; Off by default. Same value the tray's "Start with Windows" item toggles (dekan-platform::autostart),
; so either side can undo what the other set.
Name: "autostart"; Description: "{cm:AutoStartProgram,{#MyAppName}}"; GroupDescription: "{cm:AutoStartProgramGroupDescription}"; Flags: unchecked

[InstallDelete]
; ISCC warns that per-user paths in an admin install may not resolve to the desktop user's profile.
; True, and accepted: this is best effort for the common single-user case.
;
; A per-user install location must never hold a program the user can overwrite; move it out.
Type: filesandordirs; Name: "{localappdata}\Programs\Dekan"
; Backup copies in the tools folder are never loaded and only confuse the hash check.
Type: files; Name: "{app}\tools\*.orig"
Type: files; Name: "{app}\tools\*.bak"
; Overlays built by an earlier Dekan: rebuilt on demand (the cache is keyed by builder revision),
; so dropping them on upgrade frees gigabytes and costs one build.
Type: filesandordirs; Name: "{localappdata}\Dekan\overlay"

[Dirs]
; Dekan's own tools folder: where the user places the injector (LTK host + DLL), under Program Files so
; only an administrator can change it. Dekan never loads tools from another product's install.
Name: "{app}\tools"
Name: "{localappdata}\Dekan\state"
; The skin library is created empty and filled at runtime — official skins are generated from the
; installed game (Fase J), custom mods are the user's. Nothing is shipped into it (see [Files]),
; so there is no library under Program Files and no 160 MB of static .fantome in the installer.
Name: "{localappdata}\Dekan\library"

[Files]
Source: "..\dist\dekan.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\dekan.ico"; DestDir: "{app}\assets"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
Source: "..\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
; Injection backend: NOT shipped. The LTK patcher license forbids redistributing League Toolkit's signed
; binaries outside an official LTK Manager release, so the user copies ltk_patcher_host.exe and
; ltk_patcher_dll.dll from one into {app}\tools (created below). Dekan validates both by SHA-256 at runtime
; and tells the user the exact path when they are missing.
; Default party mode configuration. onlyifdoesntexist preserves user customizations on upgrade.
Source: "..\dist\party.json"; DestDir: "{localappdata}\Dekan\state"; Flags: onlyifdoesntexist
; No skin library is shipped. Official skins are generated from the installed game per patch, so
; bundling static skin packages would only bloat the installer and go stale on the next patch. The
; library folder is created empty above and filled at runtime; custom mods live in
; {localappdata}\Dekan\custom_mods. Nothing is installed into the League client: the skin is picked
; in Dekan's own window.

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\assets\dekan.ico"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\assets\dekan.ico"; Tasks: desktopicon

[Registry]
; Remove any RUNASADMIN flag on Dekan so that it runs at standard user integrity.
; Both hives: an older installer wrote HKLM, and the "Run this program as an administrator" checkbox
; of the file's Properties writes HKCU. Uninstall removes them too, leaving the registry as it was.
Root: HKLM; Subkey: "SOFTWARE\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers"; ValueType: none; ValueName: "{app}\{#MyAppExeName}"; Flags: deletevalue uninsdeletevalue
Root: HKCU; Subkey: "Software\Microsoft\Windows NT\CurrentVersion\AppCompatFlags\Layers"; ValueType: none; ValueName: "{app}\{#MyAppExeName}"; Flags: deletevalue uninsdeletevalue
; Start with Windows (optional task). HKCU because Dekan runs unelevated as the desktop user;
; an admin install elevated by the same user writes that user's hive. The quoted path is
; what the app writes too, so the tray shows the item checked.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Dekan"; ValueData: """{app}\{#MyAppExeName}"""; Flags: uninsdeletevalue; Tasks: autostart
; Always registered for removal, task or not: the entry may have been turned on later from the tray,
; and an uninstall must not leave Windows launching a program that is gone. `none` writes nothing.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Dekan"; Flags: uninsdeletevalue

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent runasoriginaluser

[UninstallDelete]
; What Dekan creates at runtime and that has no value without it: logs, state (caches, history,
; party config), the WebView2 profile, built overlays (gigabytes), generated mods, and a tools copy
; in the profile. User content (skins and custom mods) is handled in [Code]: asked, not assumed.
;
; Best effort, with the same caveat as [InstallDelete]: an admin uninstall may resolve {localappdata}
; to the elevating admin's profile, not the desktop user's, so a multi-user machine can be left with
; the real user's folder untouched. The reliable, profile-correct cleanup is the app's own, which
; resolves the desktop user by API. `cargo xtask install-audit uninstalled` checks the
; result in the single-user case.
Type: filesandordirs; Name: "{localappdata}\Dekan\logs"
Type: filesandordirs; Name: "{localappdata}\Dekan\state"
Type: filesandordirs; Name: "{localappdata}\Dekan\webview2"
Type: filesandordirs; Name: "{localappdata}\Dekan\overlay"
Type: filesandordirs; Name: "{localappdata}\Dekan\mods"
Type: filesandordirs; Name: "{localappdata}\Dekan\tools"
; The injector is copied in by the user, not installed by Setup, so Setup does not know to remove it.
Type: files; Name: "{app}\tools\ltk_patcher_host.exe"
Type: files; Name: "{app}\tools\ltk_patcher_dll.dll"
Type: dirifempty; Name: "{app}\tools"
Type: dirifempty; Name: "{app}\assets"
Type: dirifempty; Name: "{app}"

[CustomMessages]
brazilianportuguese.DeleteUserContent=Remover também as skins e os mods personalizados salvos pelo Dekan?%n%n%1%n%nEscolha "Não" para mantê-los.
english.DeleteUserContent=Also remove the skins and custom mods saved by Dekan?%n%n%1%n%nChoose "No" to keep them.

[Code]
// Skins and custom mods are the user's; the rest of what Dekan made is removed by [UninstallDelete].
// A silent uninstall keeps them (the safe default): nothing the user may want is deleted unasked.
// Same profile caveat as [UninstallDelete]: under an admin uninstall {localappdata} may be the
// admin's profile, so this prompt then lists and removes that profile's folders, not the desktop
// user's. Correct per-user cleanup is the app's job.
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
    // Only when nothing is left: never a tree delete of the data folder.
    RemoveDir(DataDir);
  end;
end;

