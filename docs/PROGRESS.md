# My-Notepad 진행 상황

> 새 대화를 시작할 때 가장 먼저 읽는 요약입니다. 판정·근거·체크리스트 원문은 [WORKPLAN.md](WORKPLAN.md)가 기준입니다.
> **마지막 갱신**: 2026-10-26 · 기준 커밋 `2218450` (`main`)

## 현재 상태

| Phase | 내용 | 상태 |
| :--- | :--- | :--- |
| Phase 0 | 기술 검증·UX 프로토타입·ADR | ✅ G0 통과 (2026-10-20) |
| Phase 1 | MVP 편집기·인코딩·파일 입출력·대용량 뷰어 | ✅ G1 통과 (2026-10-24) |
| Phase 2 | 에디터 고도화·스마트 하이라이트·사이드바 | ✅ G2 통과 (2026-10-24) |
| Phase 3 | Markdown 소스/미리보기/분할 | ✅ G3 통과 (2026-10-25) |
| **Phase 4** | **찾기/바꾸기** | **진행 중** — P4-01~P4-06·P4-UX 완료, **G4 게이트 남음** |
| Phase 5 | 배포·OS 통합 | 대기 |

- 🤖 G4 자동 검증 통과: CI Windows·macOS 녹색, 10MB 매치 카운트 최대 41.5ms, 검색 중 프레임 최대 8.82ms
- 🎨 UX 백로그 `필수`·`권장` 0건 (UX-001~008 모두 재확인 완료)

## 다음 할 일

1. **G4 macOS 검증 (사용자, MacBook)** — [macos-checklist.md §5](macos-checklist.md)
   - 준비: `git pull --rebase` → `cargo xtask gen-samples` → `cargo xtask try g4` / `try search` / `try markdown`
   - 5-1~5-4: Phase 4 기능 🍎 확인 (P4-04 `Cmd+F`·`Cmd+G`·`Cmd+Shift+G`, P4-05 `Cmd+Option+F` 포함)
   - 5-6: G4 통합 시나리오 — 정규식 일괄 치환 1703개·Undo 원복, 코드 치환 6개, 한글 `\b` 15개/7개, G1~G3 재수행
2. 회신 결과를 WORKPLAN.md §6 G4·§9.2에 기록 → 전부 통과면 **G4 통과**, Phase 4 완료 처리
3. **Phase 5 착수 전 사용자 결정 필요**: Windows 코드 서명 인증서·Apple Developer 계정 준비 여부 (미준비 시 서명 없는 배포로 갈지 승인)
4. Phase 5 첫 WU: P5-01 아이콘·버전 정보

## 사용자 대기 항목

| 항목 | 장비 | 상태 |
| :--- | :--- | :--- |
| G4 통합 시나리오 공통 항목 + Phase 4 🍎 확인 | MacBook | 대기 |
| G1-1 메모장 ANSI 한글 라운드트립 (Windows 전용) | Windows | ✅ 2026-10-26 |
| Phase 5 서명 인증서·개발자 계정 결정 | — | G4 통과 후 |

## 최근 작업 기록

> 최신이 위. 대화를 마칠 때 한 줄씩 추가하고, 10줄이 넘으면 오래된 줄은 지운다 (상세는 git log·WORKPLAN §9).

- 2026-10-26 — CLAUDE.md·PROGRESS.md 추가 (새 대화에서 바로 이어 가도록)
- 2026-10-26 — G4 준비(`cargo xtask try g4`, macos-checklist §5), G1-1 Windows 통과
- 2026-10-26 — P4-03·P4-05·P4-06 Windows 👤 통과, P4-UX 완료(반영 0건)
- 2026-10-26 — P4-01 검색 엔진·P4-02 치환 엔진·P4-04 찾기 바 (UX-6 전 항목 좋음)
- 2026-10-25 — G3 통과 (Phase 3 macOS 확인 포함), 저장소 공개 전환(CI 사용 시간)
