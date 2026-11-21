; ContextManager 설치 스크립트 (Inno Setup 6)
; build-installer.ps1이 릴리스 빌드 결과(target\release\ContextManager.exe)를 넣고 이 스크립트를 컴파일함
; 사용자별 설치: %LocalAppData%\Programs\ContextManager

#ifndef AppVersion
  #define AppVersion "1.1.7"
#endif
#ifndef BuildDir
  #define BuildDir "..\target\release"
#endif

#define AppName "ContextManager"
#define AppExe "ContextManager.exe"
#define AppPublisher "drasys"
#define AppUrl "https://github.com/workingdad365/contextmanager"
#define AppIdGuid "{9C4B6D2E-7A81-4F3C-B51D-2E8A9F4C6B10}"

[Setup]
AppId={{#AppIdGuid}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppUrl}
AppSupportURL={#AppUrl}
VersionInfoVersion={#AppVersion}
DefaultDirName={localappdata}\Programs\ContextManager
DisableDirPage=yes
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=ContextManager-Setup-{#AppVersion}
SetupIconFile=..\assets\contextmanager.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
CloseApplications=force
RestartApplications=no
ShowLanguageDialog=no

[Languages]
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
korean.LaunchAfterInstall=설치 후 {#AppName} 실행
korean.CreateDesktopIcon=바탕 화면 바로 가기 만들기
english.LaunchAfterInstall=Launch {#AppName} after install
english.CreateDesktopIcon=Create a desktop shortcut

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; Flags: unchecked

[Files]
Source: "{#BuildDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
; requireAdministrator 매니페스트가 내장되어 있으므로 shellexec 로 UAC 프롬프트를 통해 실행
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchAfterInstall}"; Flags: nowait postinstall skipifsilent shellexec
; 앱 내 업데이트(/SILENT /RELAUNCH=1)로 설치한 경우 앱을 바로 다시 실행
Filename: "{app}\{#AppExe}"; Flags: nowait shellexec; Check: IsRelaunchRequested

[UninstallRun]
; 제거 전 실행 중인 앱 종료
Filename: "{sys}\taskkill.exe"; Parameters: "/IM {#AppExe} /F"; Flags: runhidden; RunOnceId: "KillApp"

[Code]
function IsRelaunchRequested: Boolean;
begin
  Result := ExpandConstant('{param:RELAUNCH|0}') = '1';
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM {#AppExe} /F', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := '';
end;
