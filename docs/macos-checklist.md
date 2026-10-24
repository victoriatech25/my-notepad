# MacBook Pro 검증·작업 체크리스트

> 기준: [WORKPLAN.md](WORKPLAN.md) §0.8(개발 환경), P1-MAC, 각 WU의 🍎 항목 · 작성일: 2026-10-20
> 결과는 항목 번호별 ✅ / ❌(+ 현상·로그)로 회신하면 WORKPLAN §9 검증 기록에 옮긴다.

## 0. 처음 한 번: 개발 환경 준비 (P0-00 🍎)

```bash
# 1) 빌드 도구
xcode-select --install

# 2) Rust (rust-toolchain.toml이 1.98.0을 자동 설치)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 3) GitHub 인증 — 비공개 저장소이므로 victoriatech25 계정으로 로그인
brew install gh        # Homebrew가 없으면 https://cli.github.com 에서 설치
gh auth login          # GitHub.com → HTTPS → victoriatech25

# 4) 클론과 저장소 설정
mkdir -p ~/devel && cd ~/devel
git clone https://github.com/victoriatech25/my-notepad.git
cd my-notepad
git config user.name victoriatech25
git config user.email victoria.tech25@gmail.com

# 5) 점검 (첫 실행 때 Rust 1.98.0 설치·빌드로 몇 분 걸림)
cargo xtask doctor

# 6) 칩 종류 확인 → WORKPLAN §0.8 "장비 2" 칸에 기록
sysctl -n machdep.cpu.brand_string
```

- [x] **0-1** `cargo xtask doctor`가 "개발 환경 점검 통과"를 출력한다 (CRLF 픽스처 바이트 보존 포함) — 2026-10-23 통과
- [x] **0-2** 칩 종류: `Apple M1 Pro` (macOS 26.6.2)
- [x] **0-3** 장비 전환 연습: Mac에서 아무 파일(예: `docs/macos-checklist.md` 맨 아래 메모 줄)을 고쳐 커밋·`git push` → Windows에서 `git pull`로 반영 확인 — 2026-10-24 통과 (Windows에서 메모 줄 확인)

## 1. Phase 0 일괄 검증 (P1-MAC) — G1 전까지 필수

> ⚠️ **1-4(한글 IME)가 핵심**입니다. 불합격이면 ADR 0001과 UI 프레임워크 선택을 다시 검토합니다.

### 1-1 · 1-2 화면 선명도와 폰트 (P0-01, P0-02)
```bash
cargo xtask try font-report   # 폰트 로드 보고서 (창 없음)
cargo xtask try spike         # 상단에서 "폰트 (P0-02)" 화면 선택
```
- [x] **1-1** 창이 뜨고 Retina 화면에서 글자가 선명하다 — 2026-10-23 통과
- [x] **1-2** `font-report`에 Menlo, Apple SD 산돌고딕 Neo(Regular·Bold face)가 실패 없이 나온다 — 2026-10-23 통과 (Bold는 face 6, 굵기 700)
- [x] **1-3** 폰트 화면에서: 한글이 □ 없이 표시 · 영문·숫자 세로 정렬 · 비례폭 한글 자연스러움 · 굵은 한글이 실제로 굵음 — 2026-10-23 통과

### 1-4 한글 IME — macOS 기본 2벌식 (P0-03) ⚠️
```bash
cargo xtask try input         # 입력 검증 화면 (오른쪽에 IME 이벤트 로그)
```
- [x] 1-4a `한글 입력 테스트`가 누락·중복 없이 입력된다
- [x] 1-4b 조합 중인 글자가 **커서 위치에** 보인다
- [x] 1-4c 조합 중 `Enter` → 마지막 글자 확정 + 줄바꿈 1회 (**macOS에서 마지막 글자가 두 번 들어가는 문제가 알려져 있어 특히 주의**)
- [x] 1-4d 조합 중 `Space` → 마지막 글자 확정 + 공백 1개
- [x] 1-4e 조합 중 방향키·마우스 클릭 → 확정 후 이동, 글자 유실 없음
- [x] 1-4f 조합 중 `Delete(Backspace)` → 자모 단위 삭제 (`한` → `하` → `ㅎ`)
- [x] 1-4g 한/영 전환(Caps Lock 또는 `Ctrl+Space`) 후 영문, 다시 한글 정상 — winit 백포트(`third_party/winit-0.30.13`) 후 통과
- [x] 1-4h 빠르게 긴 문장을 입력해도 순서가 뒤섞이지 않는다
- 문제가 있으면 오른쪽 로그의 **복사** 버튼으로 IME 이벤트 로그를 붙여 회신

### 1-5 단축키 선처리 (P0-04) — 같은 `input` 화면
- [x] **1-5** 입력란 포커스 상태에서 `Cmd+S`, `Cmd+O`, `Cmd+W`, `Cmd+Option+F`(바꾸기), `Cmd+G`, `Cmd+Shift+V`가 커맨드 로그에 찍히고 입력란에 문자가 들어가지 않는다. `Cmd+K` 다음 `V`가 하나의 커맨드로 인식된다. `Cmd+H`는 앱 가리기(시스템 동작)로 동작한다 — 2026-10-24 통과 (한글 조합 중 `Cmd+K` → `V`는 본 앱에서 winit 수정 2·3과 `commands.rs` 수정 후 확인)

### 1-6 종료 가로채기 (P0-05) — 같은 `input` 화면
- [x] **1-6** 내용을 고친 뒤 `Cmd+Q` · 빨간 닫기 버튼 · Dock 아이콘 우클릭 → 종료 각각에서 앱이 바로 닫히지 않고 확인 대화상자가 뜨며, `취소`/`저장 안 함`이 의도대로 동작한다 — 2026-10-24: 빨간 닫기 버튼 통과, `Cmd+Q`·Dock 종료는 winit 수정 4 후 통과(처음엔 확인 없이 바로 종료), `취소`/`저장 안 함` 동작(1-6d)도 통과

### 1-7 메모리·기동 시간 (P0-07)
```bash
cargo xtask gen-samples
cargo xtask measure --runs 10     # 약 5분, 창이 여러 번 열렸다 닫힘
```
- [x] **1-7a** 측정 결과표 전체를 복사해 회신 (macOS는 RSS 기준) — 2026-10-23 측정, WORKPLAN §9.3 기록. 10MB 뷰어 메모리 미달(162MB)은 열기 경로 복사 줄여 133MB로 해결 (2026-10-24)
- [x] **1-7b** (선택) 재부팅 직후 cold 기동: `./target/release/editor-spike --renderer=glow --measure-out cold.txt` 실행 후 `cat cold.txt` — 2026-10-24 측정: `first_frame_ms=351.2` (재부팅 2분 뒤, 목표 ≤ 800ms 충족)

### 1-8 (권장) macOS 화면 의견 — 🎨 UX는 두 OS에서 받는다 (§0.7)
```bash
cargo xtask try prototype     # 전체 화면 UX 프로토타입
cargo xtask try md            # Markdown 분할 동기화·렌더러
```
- [x] **1-8** macOS에서 어색한 점(메뉴 위치, 단축키 표기 `⌘`, 글꼴 크기, 트랙패드 스크롤·동기화 체감 등)이 있으면 의견 회신 — 2026-10-24: `prototype`·`md` 모두 macOS라서 어색한 점 없음 (UX 백로그 등록 없음)

## 2. Phase 1 중 macOS 항목 (해당 WU가 끝난 뒤)
| WU | 확인 |
| :--- | :--- |
| P1-01 | CI의 macOS 작업이 녹색 (GitHub Actions — Mac 작업 불필요, 결과만 확인) — ✅ 2026-10-24 (브랜치 CI 두 OS 녹색, `main`에 병합) |
| P1-02 | 본 앱에서 1-3 폰트 항목 재확인 — ✅ 2026-10-24 |
| P1-09 | 편집·Undo(`Cmd+Z`/`Cmd+Shift+Z`)·클립보드·모두 선택·Tab 입력을 `Cmd` 단축키로 확인, 1-4 IME 재확인 — ✅ 2026-10-24 |
| P1-10 | 탭·창 메뉴·탭 이동 단축키(`Ctrl+Tab`, `Cmd+Shift+]`/`[`, `Cmd+Option+→`/`←`, `Cmd+1~9`) — ✅ 2026-10-24 |
| P1-11 | `Cmd+Q`로 미저장 확인·여러 탭 일괄 닫기 — ✅ 2026-10-24 (`Cmd+Q`·Dock 종료는 winit 수정 4 반영) |
| P1-14 | 10MB·50MB 파일 뷰어 열기·트랙패드 스크롤·복사 — ✅ 2026-10-24 (메모리 개선 `9d439fc` 반영 빌드) |

## 3. 이후 단계의 macOS 항목 (참고)
| 단계 | 항목 |
| :--- | :--- |
| G3 (Phase 3) | Markdown 분할 보기 시나리오 1회 |
| P4-04 (Phase 4) | 찾기 `Cmd+F`, 다음/이전 `Cmd+G` / `Cmd+Shift+G` |
| P5-03 (Phase 5) | `.dmg` 설치 후 Gatekeeper 경고 없음(공증 시), Apple Silicon/Intel 실행, Finder "다음으로 열기"로 `.md` 열기 |
| P5-06 (Phase 5) | `docs/manual-test.md` 전체 항목 macOS 통과 |

## 장비 전환 규칙 요약 (§0.8)
1. 작업 시작: `git pull --rebase`
2. 떠나기 전: 커밋 후 `git push` (미완성은 `wip:` 커밋)
3. 검증은 요청서에 적힌 **같은 커밋**으로

---
메모 (장비 전환 연습용):
- 2026-10-23 MacBook Pro(Apple M1 Pro)에서 커밋·push. Windows에서 `git pull`로 이 줄이 보이면 0-3 통과.
