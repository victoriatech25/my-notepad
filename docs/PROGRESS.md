# My-Notepad 진행 상황

> 새 대화를 시작할 때 가장 먼저 읽는 요약입니다. 판정·근거·체크리스트 원문은 [WORKPLAN.md](WORKPLAN.md)가 기준입니다.
> **마지막 갱신**: 2026-10-26 · `main` `4dd0d11`(P5-01 병합) · 진행 중 브랜치 `wu/P5-02-windows-installer` (P5-02, Windows 확인 후 병합)

## 현재 상태

| Phase | 내용 | 상태 |
| :--- | :--- | :--- |
| Phase 0 | 기술 검증·UX 프로토타입·ADR | ✅ G0 통과 (2026-10-20) |
| Phase 1 | MVP 편집기·인코딩·파일 입출력·대용량 뷰어 | ✅ G1 통과 (2026-10-24) |
| Phase 2 | 에디터 고도화·스마트 하이라이트·사이드바 | ✅ G2 통과 (2026-10-24) |
| Phase 3 | Markdown 소스/미리보기/분할 | ✅ G3 통과 (2026-10-25) |
| Phase 4 | 찾기/바꾸기 | ✅ G4 통과 (2026-10-26) |
| **Phase 5** | **배포·OS 통합** | **진행 중** — P5-01 완료, P5-02 구현 완료·Windows 확인 대기 |

- G4: 🤖 macOS·CI 녹색(10MB 매치 카운트 최대 41.5ms, 검색 중 프레임 최대 8.82ms) · 🍎👤 macos-checklist §5 전 항목 통과 · G1-1 Windows 통과
- 🎨 UX 백로그 `필수`·`권장` 0건 (UX-001~008 모두 재확인 완료)

## 다음 할 일

- **결정 (2026-10-26)**: 서명 없는 배포 · 제품명 **Vic Notepad**(저장소·크레이트·실행 파일·설정 폴더 이름은 유지) · 앱 아이콘 D1안(하늘색 메모지 + 파란 연필) — PLAN.md v0.5

1. **P5-02 Windows 확인** (브랜치 `wu/P5-02-windows-installer`, Inno Setup 6): `winget install JRSoftware.InnoSetup`(처음 한 번) → `cargo xtask package` → `target/dist/vic-notepad-0.1.0-windows-x64-setup.exe` 설치 → WORKPLAN P5-02 👤 7개·🎨 1개. SmartScreen 화면은 인터넷에서 받은 파일에만 나오므로 CI 아티팩트 또는 다른 경로로 받은 사본으로 확인
2. 통과하면 `main`에 병합 → P5-03 macOS 패키지(MacBook) 또는 P5-04 단일 인스턴스

## 사용자 대기 항목

| 항목 | 장비 | 상태 |
| :--- | :--- | :--- |
| G4 통합 시나리오 공통 항목 + Phase 4 🍎 확인 | MacBook | ✅ 2026-10-26 |
| G1-1 메모장 ANSI 한글 라운드트립 (Windows 전용) | Windows | ✅ 2026-10-26 |
| Phase 5 서명 인증서·개발자 계정 결정 | — | ✅ 2026-10-26 서명 없는 배포 |
| P5-01 아이콘·제품명 화면 확인 | MacBook | ✅ 2026-10-26 |
| P5-01 작업 표시줄 아이콘·실행 파일 속성 | Windows | ✅ 2026-10-26 |
| P5-02 설치·연결 프로그램·우클릭 메뉴·제거·포터블·SmartScreen | Windows | 대기 |

## 최근 작업 기록

> 최신이 위. 대화를 마칠 때 한 줄씩 추가하고, 10줄이 넘으면 오래된 줄은 지운다 (상세는 git log·WORKPLAN §9).

- 2026-10-26 — P5-02 구현: Inno Setup 설치 프로그램(사용자 단위·HKCU)·포터블 zip `cargo xtask package`, 조용한 설치·제거로 레지스트리 등록·정리 확인, `docs/install.md`
- 2026-10-26 — P5-01 Windows 👤 통과(작업 표시줄·Alt+Tab 아이콘, exe 제품명 Vic Notepad·버전 0.1.0), `main`에 병합 — **P5-01 완료**
- 2026-10-26 — Phase 5 결정(서명 없음·Vic Notepad·아이콘 D1), P5-01 구현(`cargo xtask icons`, 창 아이콘·제품명·Windows 리소스·Info.plist)
- 2026-10-26 — **G4 통과** (macOS §5 전 항목·통합 시나리오 1~3, 1703개 치환 결과는 grep·wc·cmp로 확인), Phase 4 완료
- 2026-10-26 — CLAUDE.md·PROGRESS.md 추가 (새 대화에서 바로 이어 가도록)
- 2026-10-26 — G4 준비(`cargo xtask try g4`, macos-checklist §5), G1-1 Windows 통과
- 2026-10-26 — P4-03·P4-05·P4-06 Windows 👤 통과, P4-UX 완료(반영 0건)
- 2026-10-26 — P4-01 검색 엔진·P4-02 치환 엔진·P4-04 찾기 바 (UX-6 전 항목 좋음)
- 2026-10-25 — G3 통과 (Phase 3 macOS 확인 포함), 저장소 공개 전환(CI 사용 시간)
