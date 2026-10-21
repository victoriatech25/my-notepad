# My-Notepad

Windows 기본 메모장보다 강력하고, Electron 기반 에디터보다 가벼운 **Notepad++ 스타일의 로컬 텍스트 에디터**입니다.
웹뷰 없이 순수 Rust 네이티브 GUI(`eframe`/`egui`)로 Windows와 macOS에서 동작하는 것을 목표로 합니다.

> **현재 상태**: Phase 0(기술 검증 스파이크, G0 통과) 완료 — Phase 1(MVP 기본 편집기 및 파일 입출력) 진행 중.

## 주요 목표 기능

- 멀티 탭, 폴더 탐색 사이드바
- 한글 입력(IME) 품질 — Windows / macOS 모두 기본 메모장 수준
- 인코딩 자동 감지·변환: UTF-8, UTF-8 BOM, EUC-KR(CP949), UTF-16 LE/BE
- 줄바꿈(CRLF / LF) 감지·변환
- **Markdown 소스 보기 / 미리보기 / 분할 보기** (스크롤 동기화)
- 정규식 찾기/바꾸기, 선택 단어 일괄 하이라이트

## 문서

| 문서 | 내용 |
| :--- | :--- |
| [docs/PLAN.md](docs/PLAN.md) | 기획·아키텍처 설계서 (기능 명세, 인코딩 정책, 기술 스택, 로드맵) |
| [docs/WORKPLAN.md](docs/WORKPLAN.md) | 작업계획서 (작업 단위, 검증 체크리스트, Phase 게이트, 개발 환경 규칙) |
| [docs/macos-checklist.md](docs/macos-checklist.md) | MacBook Pro 준비·검증 절차 (macOS 🍎 항목 모음) |

## 개발 환경

Windows PC와 MacBook Pro를 오가며 개발하며, 코드는 이 GitHub 저장소로만 동기화합니다.
자세한 규칙은 [WORKPLAN.md §0.8](docs/WORKPLAN.md)을 따릅니다.

### 준비

| 항목 | Windows | macOS |
| :--- | :--- | :--- |
| Rust | [rustup](https://rustup.rs) 설치 | [rustup](https://rustup.rs) 설치 |
| 빌드 도구 | Visual Studio Build Tools (C++ 워크로드) | `xcode-select --install` |
| Git 설정 | `git config core.autocrlf false` (저장소 안에서) | 기본값 유지 |

Rust 버전은 `rust-toolchain.toml`에 고정되어 있어 처음 빌드할 때 자동으로 설치됩니다.

```bash
git clone https://github.com/victoriatech25/my-notepad.git
cd my-notepad
cargo xtask doctor
```

`cargo xtask doctor`가 "개발 환경 점검 통과"를 출력하면 준비가 끝난 것입니다.

### 자주 쓰는 명령

| 명령 | 설명 |
| :--- | :--- |
| `cargo xtask help` | 개발 보조 명령과 시나리오 목록 |
| `cargo xtask try <시나리오>` | 릴리스 빌드 후 실행 (사용자 검증용) |
| `cargo xtask doctor` | 개발 환경 점검 |
| `cargo fmt --all --check` | 코드 형식 검사 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 정적 검사 |
| `cargo test --workspace` | 테스트 |

### 장비 전환 규칙 (요약)

1. 작업 시작 시 `git pull --rebase`
2. 장비를 떠나기 전 커밋하고 `git push` (미완성은 `wip:` 커밋)
3. 작업 단위마다 `wu/<WU-ID>-<설명>` 브랜치, 검증 통과 후 `main`에 병합

## 저장소 구조

```text
my-notepad/
├── docs/          기획·작업계획 문서
├── spikes/        Phase 0 실험 프로젝트
├── xtask/         개발 보조 명령 (cargo xtask)
└── tests/fixtures 테스트용 원본 파일 (바이트 그대로 보존)
```
