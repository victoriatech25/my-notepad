# My-Notepad 에이전트 작업 규칙 (AGENTS.md)

이 문서는 저장소 루트에 위치하여 모든 에이전트가 자동으로 로드하는 프로젝트 규칙입니다. 세부 기획은 [docs/PLAN.md](docs/PLAN.md), 작업 단위와 진행 절차는 [docs/WORKPLAN.md](docs/WORKPLAN.md)를 따릅니다.

## 1. 사용자 화면 검증용 GUI 실행 규칙 (중요)

Windows에서 에이전트의 `run_command`는 격리된 데스크톱(`WinSta0\exebox-...`)에서 동작하며 명령 종료 시 자식 프로세스를 함께 종료합니다.

- 사용자가 **"실행해줘"** 또는 화면 검증을 위해 앱 실행을 요청할 때는 반드시 **`WinSta0\Default` 데스크톱**에 창을 띄우고 **백그라운드 데몬(`IsDaemon: true`)으로 프로세스 종료까지 대기**해야 합니다.
- 본 저장소의 `xtask`는 Windows에서 GUI 시나리오를 실행할 때 자동으로 `WinSta0\Default` 데스크톱에 프로세스를 생성합니다. 에이전트가 직접 창을 띄울 때는 반드시 `--wait` 인자와 `IsDaemon: true`를 함께 사용하세요:
  ```powershell
  # run_command (IsDaemon: true, WaitMsBeforeAsync: 5000)
  cargo xtask try <시나리오> --wait
  ```
- 만약 `xtask`를 거치지 않고 임의의 GUI 실행 파일을 띄워야 한다면, Win32 `CreateProcessW`에서 `STARTUPINFO.lpDesktop = "WinSta0\\Default"`를 지정하고 `Wait-Process`로 대기해야 사용자 화면에 창이 보입니다.
- **Windows 파일 탐색기(`explorer.exe`)를 열 때**: `explorer.exe`를 직접 실행하면 샌드박스 격리로 인해 창이 열리지 않고 즉시 종료됩니다. 반드시 `(New-Object -ComObject Shell.Application).Open("C:\devel\My-Notepad")` COM 호출을 사용해 사용자의 기본 데스크톱 셸에서 탐색기 창이 열리게 하세요.

## 2. 개발 및 브랜치 규칙 요약

- **WU 브랜치**: 각 작업 단위는 `wu/<WU-ID>-<설명>` 브랜치에서 작업하고, 검증 완료 후 `main`에 병합합니다.
- **줄바꿈 보존**: `tests/fixtures/**`와 `spikes/samples/**`의 바이트가 변형되지 않도록 Windows에서는 `git config core.autocrlf false`를 유지합니다 (`cargo xtask doctor`로 점검).
- **공통 완료 조건 (DoD)**:
  1. `cargo build` (경고 0)
  2. `cargo clippy --workspace --all-targets -- -D warnings` 통과
  3. `cargo fmt --all --check` 통과
  4. `cargo test --workspace` 전체 통과
