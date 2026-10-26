# My-Notepad — Claude Code 작업 안내 (CLAUDE.md)

Claude Code가 새 대화를 시작할 때마다 자동으로 읽는 파일입니다. 공통 규칙은 AGENTS.md를 그대로 따릅니다.

@AGENTS.md

## 1. 새 대화를 시작하면 먼저 읽을 것

1. [docs/PROGRESS.md](docs/PROGRESS.md) — **지금 어디까지 했고 다음에 무엇을 하는지** (짧은 요약)
2. 필요할 때만: [docs/WORKPLAN.md](docs/WORKPLAN.md)의 해당 WU 절 — 1,200줄이 넘으므로 전체를 읽지 말고 §1 현황판과 진행 중인 Phase 절만 읽는다
3. 기능 정의·정책이 필요하면 [docs/PLAN.md](docs/PLAN.md), 화면 배치·문구·단축키는 [docs/ux/ui-spec.md](docs/ux/ui-spec.md)

"진행사항 파악해줘" 같은 요청을 받으면 PROGRESS.md와 `git log --oneline -10`, `git status`만으로 답하고, 그 이상은 필요한 부분만 찾아 읽는다.

## 2. 문서 지도

| 문서 | 역할 | 언제 고치나 |
| :--- | :--- | :--- |
| `docs/PROGRESS.md` | 현재 상태·다음 할 일·사용자 대기 항목 요약 | **대화를 마칠 때마다** |
| `docs/WORKPLAN.md` | WU 정의, 체크리스트, §9 검증 기록·UX 백로그 (기준 문서) | WU 상태·검증 결과가 바뀔 때 |
| `docs/PLAN.md` | 기획·아키텍처·정책 | 결정이 바뀔 때 (WORKPLAN보다 먼저) |
| `docs/macos-checklist.md` | 🍎 MacBook에서 할 검증 절차 | macOS 검증 항목이 생길 때 |
| `docs/ux/ui-spec.md` | 확정된 UX 결정 | 🎨 결정이 확정될 때 |
| `docs/adr/` | 방식 결정 기록 | 큰 기술 결정 시 |

PROGRESS.md는 요약이고, 판정·근거는 WORKPLAN.md에 남긴다. 두 문서가 다르면 WORKPLAN.md가 맞다.

## 3. 작업 흐름 (WORKPLAN §0 요약)

- WU 단위로 진행하고, Phase별 "수행 순서"를 따른다. `wu/<WU-ID>-<설명>` 브랜치에서 작업 후 검증 통과 시 `main`에 병합한다.
- 👤(기능)·🎨(UX)·🍎(macOS) 항목은 **사용자 회신 없이 완료 처리하지 않는다.** 검증 요청에는 커밋 해시와 `cargo xtask try <시나리오>` 한 줄 실행 방법을 적는다 (WORKPLAN §9.1 양식).
- 화면이 있는 기능은 화면을 먼저 만들어 🎨 확인을 받고 내부를 채운다.
- 완료 조건: `cargo build`(경고 0) · `cargo clippy --workspace --all-targets -- -D warnings` · `cargo fmt --all --check` · `cargo test --workspace`
- 개발 보조 명령은 `cargo xtask`로만 만든다 (`.ps1`·`.sh` 금지). 시나리오 목록은 `cargo xtask help`.

## 4. Claude Code에서 앱 실행하기 (Windows)

AGENTS.md §1의 `run_command`·`IsDaemon` 안내는 다른 에이전트 도구 기준이다. Claude Code에서는 같은 명령을 **백그라운드 실행**(`run_in_background`)으로 띄운다.

```powershell
cargo xtask try <시나리오> --wait
```

`xtask`가 `WinSta0\Default` 데스크톱에 창을 만들므로 사용자 화면에 보인다. 탐색기 창은 `explorer.exe` 대신 `(New-Object -ComObject Shell.Application).Open("<경로>")`로 연다.

## 5. Git·장비 전환

- 작업 시작: `git pull --rebase` · 장비를 떠나기 전: 커밋 후 `git push` (미완성은 WU 브랜치에 `wip:` 커밋)
- `core.autocrlf false` 유지, `tests/fixtures/**`는 바이트 그대로 보존 (`cargo xtask doctor`로 점검)
- **Windows PC에서 push**: 기본 gh 계정(`kyhoon-hj`)은 이 저장소에 push할 수 없다.
  1. `gh auth switch --hostname github.com --user victoriatech25` (이미 활성이면 생략)
  2. `git push`
  3. `gh auth switch --hostname github.com --user kyhoon-hj` 로 되돌리기

## 6. 대화를 마칠 때

사용자가 작업을 마치거나 "정리해줘"라고 하면:

1. `docs/PROGRESS.md`의 "현재 상태", "다음 할 일", "사용자 대기 항목", "최근 작업 기록"을 갱신한다
2. WORKPLAN.md의 WU 상태·§9 기록이 바뀌었으면 함께 갱신한다
3. 커밋하고, 사용자가 원하면 push한다 (§5 절차)
