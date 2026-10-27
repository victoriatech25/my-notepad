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

배포 파일은 `vic-notepad-<버전>-macos-universal.dmg` 하나입니다. Apple Silicon·Intel Mac 모두에서 실행되며 macOS 11 이상이 필요합니다.

### 설치

1. `.dmg`를 열고 **Vic Notepad**를 옆의 **Applications** 폴더로 끌어다 놓습니다.
2. 응용 프로그램 폴더에서 Vic Notepad를 처음 열면 **"Apple은 'Vic Notepad'에 악성 코드가 없음을 확인할 수 없습니다"**(또는 "확인되지 않은 개발자") 경고가 나옵니다. 개발자 인증서로 서명·공증하지 않은 앱이라 처음 한 번 나옵니다. 아래 중 하나로 넘깁니다.
   - **macOS 15 이상**: 경고에서 **완료**를 누른 뒤 **시스템 설정 → 개인정보 보호 및 보안**을 열고, 아래쪽 보안 항목의 "'Vic Notepad'이(가) 차단되었습니다" 옆 **그래도 열기**를 누릅니다. 다시 뜨는 확인 창에서 **그래도 열기**를 누르고 암호(또는 Touch ID)를 입력합니다.
   - **macOS 14 이하**: Finder에서 Vic Notepad를 **우클릭(Control+클릭) → 열기**를 누르고, 확인 창에서 **열기**를 누릅니다.
3. 한 번 열고 나면 이후에는 경고 없이 실행됩니다.

### 파일 연결

설치하면 텍스트(.txt 등)·Markdown(.md)·로그(.log) 파일의 **다음으로 열기** 목록에 Vic Notepad가 들어갑니다. 기본 앱은 바뀌지 않습니다. 기본 앱으로 쓰려면 파일을 선택하고 **정보 가져오기(⌘I) → 다음으로 열기 → Vic Notepad → 모두 변경…**을 누릅니다.

이미 떠 있는 Vic Notepad가 있으면 Finder에서 연 파일은 그 창의 새 탭으로 열립니다. Dock 아이콘에 파일을 끌어다 놓아도 됩니다.

### 제거

응용 프로그램 폴더의 Vic Notepad를 휴지통으로 옮깁니다. 설정은 `~/Library/Application Support/My-Notepad`, 로그는 `~/Library/Logs/My-Notepad`에 남으므로 필요 없으면 함께 지웁니다.

### 만들기 (개발자)

```bash
cargo xtask package   # → target/dist/ 에 Vic Notepad.app·vic-notepad-<버전>-macos-universal.dmg
```

`x86_64-apple-darwin` Rust 대상이 없으면 처음 실행할 때 `rustup target add`로 받습니다. 서명은 ad-hoc(`codesign -s -`)입니다. Apple Silicon은 서명이 전혀 없는 실행 파일을 돌리지 않고, 번들 서명이 없으면 내려받은 앱이 "손상되었습니다"로 막혀 **그래도 열기**로도 넘길 수 없기 때문입니다.
