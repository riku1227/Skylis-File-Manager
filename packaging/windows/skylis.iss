; Keep this AppId forever so upgrades reuse the existing install and uninstall entry.
#define SkylisAppId "{22BED384-9D23-4ABF-B68E-2EA468DF77C6}"

[Setup]
AppId={{#SkylisAppId}
AppName=Skylis
AppVersion={#AppVersion}
AppPublisher=Skylis
VersionInfoVersion={#NumericVersion}
VersionInfoTextVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\Skylis
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UsePreviousAppDir=yes
DisableDirPage=auto
CloseApplications=yes
RestartApplications=no
SourceDir=..\..
OutputDir=target\installer
OutputBaseFilename=Skylis-{#AppVersion}-x64-setup
SetupIconFile=apps\explorer\assets\branding\skylis.ico
UninstallDisplayIcon={app}\skylis.exe

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#ReleaseExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#LicenseNotices}"; DestDir: "{app}"; Flags: ignoreversion
Source: "LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "licenses\GPUI-APACHE-2.0.txt"; DestDir: "{app}\licenses"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\Skylis"; Filename: "{app}\skylis.exe"
Name: "{userdesktop}\Skylis"; Filename: "{app}\skylis.exe"; Tasks: desktopicon

[CustomMessages]
english.CloseSkylis=Close all Skylis windows, then click Retry to continue uninstalling.
japanese.CloseSkylis=Skylis のウィンドウをすべて閉じてから、［再試行］をクリックしてアンインストールを続けてください。
english.KeepInstallDir=An existing Skylis installation was found. To preserve its Windows folder integration, install the update in the existing directory: %1
japanese.KeepInstallDir=Skylis は既にインストールされています。Windows のフォルダー連携を維持するため、更新は既存のフォルダーにインストールしてください。%1
english.IntegrationRestoreFailed=Some Windows folder integration may remain. To remove it, reinstall Skylis, open Settings > "Windows連携" (Windows integration), choose "元の設定に戻す" (Restore original settings) for each item, then uninstall again.
japanese.IntegrationRestoreFailed=Windows のフォルダー連携が一部残っている可能性があります。Skylis を再インストールし、設定の「Windows連携」で「元の設定に戻す」を選んでから、もう一度アンインストールしてください。

[Code]
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  PreviousDir: String;
begin
  Result := '';
  { Also protect the executable path when /DIR overrides the wizard default. }
  if RegQueryStringValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{#SkylisAppId}_is1',
    'InstallLocation', PreviousDir) and (PreviousDir <> '') then
    if CompareText(RemoveBackslashUnlessRoot(PreviousDir), RemoveBackslashUnlessRoot(WizardDirValue)) <> 0 then
      Result := FmtMessage(CustomMessage('KeepInstallDir'), [PreviousDir]);
end;

function InitializeUninstall(): Boolean;
begin
  { Leave Setup free to use Restart Manager. Uninstall waits for Skylis to close. }
  while CheckForMutexes('Local\Skylis.Running') do
  begin
    Log('Skylis is running. Close it before uninstalling.');
    Result := False;
    if UninstallSilent then
      Exit;
    if MsgBox(CustomMessage('CloseSkylis'), mbError, MB_RETRYCANCEL) <> IDRETRY then
      Exit;
  end;
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  ResultCode: Integer;
begin
  if CurUninstallStep <> usUninstall then
    Exit;

  Log('Restoring Windows folder integration with skylis.exe --restore-integration.');
  if not Exec(ExpandConstant('{app}\skylis.exe'), '--restore-integration',
    ExpandConstant('{app}'), SW_HIDE, ewWaitUntilTerminated, ResultCode) then
    Log(Format('Could not start folder integration restore (Windows error %d).', [ResultCode]))
  else
  begin
    Log(Format('Folder integration restore exited with code %d.', [ResultCode]));
    if ResultCode = 0 then
      Exit;
  end;

  if not UninstallSilent then
    MsgBox(CustomMessage('IntegrationRestoreFailed'), mbError, MB_OK);
end;
