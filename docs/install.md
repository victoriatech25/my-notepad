# Vic Notepad 설치 안내

Vic Notepad는 코드 서명 없이 배포합니다 (PLAN.md Phase 5 결정). 그래서 처음 실행할 때 OS가 경고를 보여 주며, 아래 절차로 넘어가면 이후에는 경고 없이 실행됩니다.

## Windows

배포 파일 두 가지 중 하나를 고릅니다.

| 파일 | 용도 |
| :--- | :--- |
| `vic-notepad-<버전>-windows-x64-setup.exe` | 설치 프로그램 — 시작 메뉴, 연결 프로그램, 탐색기 우클릭 메뉴까지 등록 |
| `vic-notepad-<버전>-windows-x64-portable.zip` | 포터블 — 압축을 풀고 `Vic Notepad\my-notepad.exe`를 바로 실행 (아무것도 등록하지 않음) |

### 설치 프로그램

1. `…-setup.exe`를 실행합니다. 관리자 권한은 필요 없습니다 (현재 사용자에게만 설치, 기본 위치 `%LOCALAPPDATA%\Programs\Vic Notepad`).
2. **"Windows의 PC 보호"(SmartScreen) 화면이 나오면** → **추가 정보** → **실행**을 누릅니다. 인터넷에서 받은 서명 없는 파일이라 처음 한 번 나옵니다.
3. 추가 작업을 고릅니다.
   - 텍스트 파일(.txt·.md·.log)의 "연결 프로그램" 목록에 추가 — 기본 앱은 바뀌지 않습니다. 기본 앱으로 쓰려면 파일 우클릭 → 연결 프로그램 → 다른 앱 선택 → Vic Notepad → "항상"
   - 탐색기 우클릭 메뉴에 "Vic Notepad로 편집" 추가 — Windows 11에서는 우클릭 → **추가 옵션 표시** 안에 있습니다 (Windows 10은 첫 메뉴)
   - 바탕 화면 바로 가기 (기본 꺼짐)

### 제거

설정 → 앱 → 설치된 앱 → Vic Notepad → 제거. 등록한 연결 프로그램·우클릭 메뉴·시작 메뉴 항목을 모두 지우고, 마지막에 설정·로그 폴더(`%APPDATA%\My-Notepad`)를 지울지 묻습니다. [아니요]를 고르면 다시 설치했을 때 설정과 최근 파일이 그대로 남습니다.

### 만들기 (개발자)

```powershell
winget install JRSoftware.InnoSetup   # 한 번만
cargo xtask package                   # → target/dist/ 에 setup.exe·portable.zip
```

## macOS

P5-03에서 추가합니다 (`.dmg`, 첫 실행은 우클릭 → 열기).
