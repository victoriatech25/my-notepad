; Vic Notepad Windows 설치 프로그램 (Inno Setup 6, P5-02)
;
; 직접 컴파일하지 않고 `cargo xtask package`로 만든다 — 릴리스 빌드 뒤 버전·실행 파일 경로를 넘겨 ISCC를 부른다.
; - 사용자 단위 설치(관리자 권한 없음): %LOCALAPPDATA%\Programs\Vic Notepad, 레지스트리는 HKCU만 쓴다 (PLAN.md Phase 5 B안)
; - 연결 프로그램(.txt·.md·.log)은 "연결 프로그램" 목록에 올리기만 하고 기본 앱은 바꾸지 않는다
; - 탐색기 우클릭 "Vic Notepad로 편집"은 모든 파일(*)에 등록 — Windows 11은 "추가 옵션 표시" 안에 나온다
; - 제거하면 위 등록을 모두 지우고, 설정 폴더(%APPDATA%\My-Notepad)는 지울지 묻는다
; 이 파일은 UTF-8(BOM)으로 저장한다 — ISCC가 BOM으로 UTF-8을 알아본다.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef AppExe
  #define AppExe "..\..\target\release\my-notepad.exe"
#endif

#define AppName "Vic Notepad"
#define ExeName "my-notepad.exe"
; 우클릭 메뉴·ProgID 키 이름 (사용자에게 보이지 않는다)
#define ProgPrefix "VicNotepad"

[Setup]
; AppId는 바꾸지 않는다 — 바꾸면 이전 버전 위에 덮어 설치(업그레이드)로 인식하지 못한다
AppId={{83394C3B-B351-4D41-B734-0A18601F73EF}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=VictoriaTech
AppPublisherURL=https://github.com/victoriatech25/my-notepad
AppSupportURL=https://github.com/victoriatech25/my-notepad/issues
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} 설치 프로그램
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#AppName}
DisableProgramGroupPage=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
ChangesAssociations=yes
CloseApplications=yes
SetupIconFile=..\icons\vic-notepad.ico
UninstallDisplayIcon={app}\{#ExeName}
UninstallDisplayName={#AppName}
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
OutputBaseFilename=vic-notepad-{#AppVersion}-windows-x64-setup

[Languages]
Name: "korean"; MessagesFile: "compiler:Languages\Korean.isl"

[CustomMessages]
korean.TaskAssoc=텍스트 파일(.txt·.md·.log)의 "연결 프로그램" 목록에 추가 (기본 앱은 바꾸지 않음)
korean.TaskContextMenu=탐색기 우클릭 메뉴에 "Vic Notepad로 편집" 추가
korean.ContextMenuVerb=Vic Notepad로 편집
korean.TypeTxt=텍스트 문서
korean.TypeMd=Markdown 문서
korean.TypeLog=로그 파일
korean.AppDescription=가볍고 빠른 텍스트·Markdown 편집기
korean.DeleteSettings=설정과 로그 폴더도 지울까요?%n%n%1%n%n[아니요]를 누르면 다시 설치했을 때 창 위치·최근 파일·설정이 그대로 남아 있습니다.

[Tasks]
Name: "assoc"; Description: "{cm:TaskAssoc}"
Name: "contextmenu"; Description: "{cm:TaskContextMenu}"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#AppExe}"; DestDir: "{app}"; DestName: "{#ExeName}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#ExeName}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#ExeName}"; Tasks: desktopicon

[Registry]
; ── 연결 프로그램 (Tasks: assoc) ──
; 실행 파일 등록: "연결 프로그램 → 다른 앱 선택" 목록의 이름과 지원 형식
Root: HKA; Subkey: "Software\Classes\Applications\{#ExeName}"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "{#AppName}"; Flags: uninsdeletekey; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Applications\{#ExeName}\DefaultIcon"; ValueType: string; ValueData: """{app}\{#ExeName}"",0"; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Applications\{#ExeName}\shell\open\command"; ValueType: string; ValueData: """{app}\{#ExeName}"" -- ""%1"""; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Applications\{#ExeName}\SupportedTypes"; ValueType: string; ValueName: ".txt"; ValueData: ""; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Applications\{#ExeName}\SupportedTypes"; ValueType: string; ValueName: ".md"; ValueData: ""; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\Applications\{#ExeName}\SupportedTypes"; ValueType: string; ValueName: ".log"; ValueData: ""; Tasks: assoc
; 확장자별 ProgID — 사용자가 Vic Notepad를 기본 앱으로 고르면 이 이름·아이콘·명령을 쓴다
Root: HKA; Subkey: "Software\Classes\{#ProgPrefix}.txt"; ValueType: string; ValueData: "{cm:TypeTxt}"; Flags: uninsdeletekey; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\{#ProgPrefix}.txt\DefaultIcon"; ValueType: string; ValueData: """{app}\{#ExeName}"",0"; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\{#ProgPrefix}.txt\shell\open\command"; ValueType: string; ValueData: """{app}\{#ExeName}"" -- ""%1"""; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\{#ProgPrefix}.md"; ValueType: string; ValueData: "{cm:TypeMd}"; Flags: uninsdeletekey; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\{#ProgPrefix}.md\DefaultIcon"; ValueType: string; ValueData: """{app}\{#ExeName}"",0"; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\{#ProgPrefix}.md\shell\open\command"; ValueType: string; ValueData: """{app}\{#ExeName}"" -- ""%1"""; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\{#ProgPrefix}.log"; ValueType: string; ValueData: "{cm:TypeLog}"; Flags: uninsdeletekey; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\{#ProgPrefix}.log\DefaultIcon"; ValueType: string; ValueData: """{app}\{#ExeName}"",0"; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\{#ProgPrefix}.log\shell\open\command"; ValueType: string; ValueData: """{app}\{#ExeName}"" -- ""%1"""; Tasks: assoc
; 확장자 → ProgID: 기본값(현재 기본 앱)은 건드리지 않고 OpenWithProgids에 값만 더한다
Root: HKA; Subkey: "Software\Classes\.txt\OpenWithProgids"; ValueType: string; ValueName: "{#ProgPrefix}.txt"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.md\OpenWithProgids"; ValueType: string; ValueName: "{#ProgPrefix}.md"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
Root: HKA; Subkey: "Software\Classes\.log\OpenWithProgids"; ValueType: string; ValueName: "{#ProgPrefix}.log"; ValueData: ""; Flags: uninsdeletevalue; Tasks: assoc
; 설정 → 기본 앱 목록에 나오게 하는 등록
Root: HKA; Subkey: "Software\{#ProgPrefix}"; Flags: uninsdeletekey; Tasks: assoc
Root: HKA; Subkey: "Software\{#ProgPrefix}\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "{#AppName}"; Tasks: assoc
Root: HKA; Subkey: "Software\{#ProgPrefix}\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "{cm:AppDescription}"; Tasks: assoc
Root: HKA; Subkey: "Software\{#ProgPrefix}\Capabilities\FileAssociations"; ValueType: string; ValueName: ".txt"; ValueData: "{#ProgPrefix}.txt"; Tasks: assoc
Root: HKA; Subkey: "Software\{#ProgPrefix}\Capabilities\FileAssociations"; ValueType: string; ValueName: ".md"; ValueData: "{#ProgPrefix}.md"; Tasks: assoc
Root: HKA; Subkey: "Software\{#ProgPrefix}\Capabilities\FileAssociations"; ValueType: string; ValueName: ".log"; ValueData: "{#ProgPrefix}.log"; Tasks: assoc
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "{#AppName}"; ValueData: "Software\{#ProgPrefix}\Capabilities"; Flags: uninsdeletevalue; Tasks: assoc

; ── 탐색기 우클릭 메뉴 (Tasks: contextmenu) ──
; 여러 파일을 고르면 탐색기가 파일마다 실행한다 (Player: 15개 넘게 골라도 메뉴가 사라지지 않음)
Root: HKA; Subkey: "Software\Classes\*\shell\{#ProgPrefix}.Edit"; ValueType: string; ValueName: "MUIVerb"; ValueData: "{cm:ContextMenuVerb}"; Flags: uninsdeletekey; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\*\shell\{#ProgPrefix}.Edit"; ValueType: string; ValueName: "Icon"; ValueData: """{app}\{#ExeName}"",0"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\*\shell\{#ProgPrefix}.Edit"; ValueType: string; ValueName: "MultiSelectModel"; ValueData: "Player"; Tasks: contextmenu
Root: HKA; Subkey: "Software\Classes\*\shell\{#ProgPrefix}.Edit\command"; ValueType: string; ValueData: """{app}\{#ExeName}"" -- ""%1"""; Tasks: contextmenu

[Run]
Filename: "{app}\{#ExeName}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
// 제거가 끝나면 설정·로그 폴더를 지울지 묻는다 (조용한 제거는 남겨 둔다)
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  SettingsDir: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    SettingsDir := ExpandConstant('{userappdata}\My-Notepad');
    if DirExists(SettingsDir) and not UninstallSilent then
      if MsgBox(FmtMessage(CustomMessage('DeleteSettings'), [SettingsDir]), mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES then
        DelTree(SettingsDir, True, True, True);
  end;
end;
