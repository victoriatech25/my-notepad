//! Windows·macOS 공통 개발 보조 명령.
//!
//! 사용법: `cargo xtask <명령> [인자]` (별칭은 `.cargo/config.toml`)

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

mod icons;
mod measure;
mod package;
mod samples;

/// `cargo xtask try <시나리오>`로 실행할 수 있는 시나리오.
struct Scenario {
    name: &'static str,
    package: &'static str,
    args: &'static [&'static str],
    description: &'static str,
    /// true면 끝날 때까지 기다려 출력을 보여준다. false면 창을 띄우고 바로 돌아온다
    /// (xtask가 앱을 붙잡고 있으면 다음 빌드에서 xtask.exe를 교체하지 못한다).
    wait: bool,
    /// 실행 전에 `target/try-<시나리오>/`로 새로 복사해 인자로 넘길 파일 (저장소 기준 경로).
    /// 검증 중에 저장해도 `tests/fixtures`의 바이트가 바뀌지 않게 한다.
    /// 폴더를 적으면 같은 이름으로 통째로 복사만 하고 인자로는 넘기지 않는다 (문서가 참조하는 이미지 등).
    work_files: &'static [&'static str],
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "empty",
        package: "my-notepad",
        args: &[],
        description: "P1-01 본 앱 빈 창 실행 (메뉴바·상태 표시줄 자리 표시)",
        wait: false,
        work_files: &[],
    },
    Scenario {
        name: "basic",
        package: "my-notepad",
        args: &[],
        description: "UX-2 첫 실사용 빌드 (P1-09 에디터 + P1-10 멀티 탭·파일 열기/저장)",
        wait: false,
        work_files: &[],
    },
    Scenario {
        name: "encoding",
        package: "my-notepad",
        args: &[],
        description: "UX-3 인코딩·줄바꿈 UI (픽스처 사본 6개를 탭으로 열기: UTF-8·BOM·CP949·UTF-16 LE·손상·CRLF)",
        wait: false,
        work_files: &[
            "tests/fixtures/encoding/utf8.txt",
            "tests/fixtures/encoding/utf8_bom.txt",
            "tests/fixtures/encoding/cp949.txt",
            "tests/fixtures/encoding/utf16le.txt",
            "tests/fixtures/encoding/cp949_damaged.bin",
            "tests/fixtures/line_ending/crlf.txt",
        ],
    },
    Scenario {
        name: "large",
        package: "my-notepad",
        args: &[
            "spikes/samples/text-1mb.txt",
            "spikes/samples/text-10mb.txt",
            "spikes/samples/text-50mb.txt",
        ],
        description: "P1-14 대용량: 1MB(안내 막대)·10MB·50MB(읽기 전용 뷰어) 탭 (샘플: cargo xtask gen-samples)",
        wait: false,
        work_files: &[],
    },
    Scenario {
        name: "gutter",
        package: "my-notepad",
        args: &[
            "spikes/samples/long-lines.txt",
            "spikes/samples/lines-12k.txt",
        ],
        description: "P2-01 줄 번호 Gutter·자동 줄바꿈 (긴 줄 섞인 300줄, 12,000줄 탭 — 샘플: cargo xtask gen-samples)",
        wait: false,
        work_files: &[],
    },
    Scenario {
        name: "highlight",
        package: "my-notepad",
        args: &["spikes/samples/text-1mb.txt"],
        description: "P2-04 선택 단어 강조 (한글 확인 문서 사본 + 1MB 로그 — 샘플: cargo xtask gen-samples)",
        wait: false,
        work_files: &["tests/fixtures/editor/highlight_ko.txt"],
    },
    Scenario {
        name: "sidebar",
        package: "my-notepad",
        args: &["."],
        description: "P2-06 폴더 탐색 사이드바 (이 저장소 폴더를 열기 — .git 숨김, target 흐리게, 아이콘)",
        wait: false,
        work_files: &[],
    },
    Scenario {
        name: "tabs",
        package: "my-notepad",
        args: &["spikes/samples/lines-12k.txt"],
        description: "P2-02 확대/축소·줄 이동·탭 너비 (12,000줄 + 탭 들여쓰기 문서 사본 — 샘플: cargo xtask gen-samples)",
        wait: false,
        work_files: &["tests/fixtures/editor/tabs.txt"],
    },
    Scenario {
        name: "external",
        package: "my-notepad",
        args: &[],
        description: "P2-07 외부 변경 감지 (target/try-external/의 문서 사본 2개 — 메모장으로 고치거나 탐색기에서 지워 보기)",
        wait: false,
        work_files: &[
            "tests/fixtures/editor/highlight_ko.txt",
            "tests/fixtures/editor/tabs.txt",
        ],
    },
    Scenario {
        name: "search",
        package: "my-notepad",
        args: &[
            "spikes/samples/text-1mb.txt",
            "spikes/samples/text-10mb.txt",
        ],
        description: "P4-04 찾기 바·P4-03 백그라운드 검색·P4-05 바꾸기 (찾기 확인 문서 사본 + 1MB 로그 + 10MB 로그(읽기 전용 뷰어) — 샘플: cargo xtask gen-samples)",
        wait: false,
        work_files: &["tests/fixtures/editor/search_ko.txt"],
    },
    Scenario {
        name: "g4",
        package: "my-notepad",
        args: &[],
        description: "G4 검색·치환 통합 시나리오 (target/try-g4/의 1MB 로그·코드(search.rs)·한글 확인 문서 사본 — 저장해도 원본은 그대로, 샘플: cargo xtask gen-samples)",
        wait: false,
        work_files: &[
            "spikes/samples/text-1mb.txt",
            "src/core/search.rs",
            "tests/fixtures/editor/search_ko.txt",
        ],
    },
    Scenario {
        name: "markdown",
        package: "my-notepad",
        args: &[
            "spikes/samples/md-1mb.md",
            "target/try-markdown/links/index.md",
        ],
        description: "P3-02~ Markdown 미리보기 (gfm_all.md·korean.md·tables.md·images.md(+images/)·links/(index.md 탭)·docs/PLAN.md 사본 + 1MB 샘플을 분할 모드 탭으로 열기 — 샘플: cargo xtask gen-samples)",
        wait: false,
        work_files: &[
            "tests/fixtures/markdown/gfm_all.md",
            "tests/fixtures/markdown/korean.md",
            "tests/fixtures/markdown/tables.md",
            "tests/fixtures/markdown/images.md",
            "tests/fixtures/markdown/images",
            "tests/fixtures/markdown/links",
            "docs/PLAN.md",
        ],
    },
    Scenario {
        name: "editor",
        package: "my-notepad",
        args: &[],
        description: "P1-09 기본 에디터 (빈 새 문서 1개 — 한글 IME·Tab·Undo/Redo·클립보드 확인)",
        wait: false,
        work_files: &[],
    },
    Scenario {
        name: "spike",
        package: "editor-spike",
        args: &[],
        description: "에디터 스파이크 (입력 검증 + 폰트 화면)",
        wait: false,
        work_files: &[],
    },
    Scenario {
        name: "input",
        package: "editor-spike",
        args: &[],
        description: "P0-03~05 한글 IME·단축키·창 닫기 검증 화면",
        wait: false,
        work_files: &[],
    },
    Scenario {
        name: "fonts",
        package: "editor-spike",
        args: &[],
        description: "P0-02 폰트 Fallback 체인 확인 화면",
        wait: false,
        work_files: &[],
    },
    Scenario {
        name: "prototype",
        package: "ux-prototype",
        args: &[],
        description: "P0-UX1 전체 화면 UX 프로토타입 (가짜 데이터)",
        wait: false,
        work_files: &[],
    },
    Scenario {
        name: "perf",
        package: "editor-spike",
        args: &["--screen=perf"],
        description: "P0-06 대용량 텍스트 성능 화면 (샘플: cargo xtask gen-samples)",
        wait: false,
        work_files: &[],
    },
    Scenario {
        name: "md",
        package: "md-spike",
        args: &[],
        description: "P0-08 Markdown 렌더러 비교 (A egui_commonmark | B 자체 렌더러, docs/PLAN.md)",
        wait: false,
        work_files: &[],
    },
    Scenario {
        name: "md-block-map",
        package: "md-spike",
        args: &["--block-map"],
        description: "P0-08 B 렌더러 블록 위치표 출력·점검 (창 없음)",
        wait: true,
        work_files: &[],
    },
    Scenario {
        name: "perf-bench",
        package: "editor-spike",
        args: &["--perf-bench"],
        description: "P0-06 대용량 텍스트 자동 벤치 (창 없음, 수 분 소요)",
        wait: true,
        work_files: &[],
    },
    Scenario {
        name: "font-report",
        package: "editor-spike",
        args: &["--font-report"],
        description: "P0-02 폰트 로드 보고서만 출력 (창 없음)",
        wait: true,
        work_files: &[],
    },
];

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("try") => {
            let wait_flag = args.iter().skip(1).any(|a| a == "--wait");
            let name = args
                .iter()
                .skip(1)
                .find(|a| !a.starts_with('-'))
                .map(String::as_str);
            try_scenario(name, wait_flag)
        }
        Some("doctor") => doctor(),
        Some("gen-samples") => samples::generate(&workspace_root()),
        Some("icons") => icons::generate(&workspace_root()),
        Some("package") => package::run(&workspace_root()),
        Some("measure") => {
            let runs = args
                .iter()
                .position(|a| a == "--runs")
                .and_then(|i| args.get(i + 1))
                .and_then(|v| v.parse().ok())
                .unwrap_or(10);
            if args.iter().any(|a| a == "--viewer") {
                measure::run_viewer(&workspace_root(), runs.min(5))
            } else {
                measure::run(&workspace_root(), runs)
            }
        }
        None | Some("help" | "-h" | "--help") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(format!(
            "알 수 없는 명령: {other}\n`cargo xtask help`로 목록을 확인하세요."
        )),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("오류: {message}");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!("My-Notepad 개발 보조 명령\n");
    println!("사용법: cargo xtask <명령> [인자]\n");
    println!("명령:");
    println!(
        "  try <시나리오> [--wait]  릴리스 빌드 후 실행 (사용자 검증용, --wait는 창 종료까지 대기)"
    );
    println!("  doctor           개발 환경 점검 (두 장비 준비 상태 확인)");
    println!("  gen-samples      성능 측정용 샘플 생성 (spikes/samples, git 제외)");
    println!(
        "  icons            앱 아이콘 SVG → .ico·.icns·창 아이콘 PNG (assets/icons, 커밋 대상)"
    );
    println!(
        "  package          배포 파일 만들기 → target/dist (Windows: 설치 프로그램·포터블 zip, Inno Setup 6 필요)"
    );
    println!("  measure [--runs N]  메모리·기동 시간 측정 (렌더러 × 한글 폰트 × 문서, 기본 10회)");
    println!(
        "  measure --viewer [--runs N]  본 앱 대용량 뷰어 10MB·50MB: 첫 화면·스크롤 프레임·메모리 (P1-14, 기본 3회)"
    );
    println!("  help             이 도움말\n");
    println!("시나리오:");
    for scenario in SCENARIOS {
        println!("  {:<16} {}", scenario.name, scenario.description);
    }
}

fn try_scenario(name: Option<&str>, wait_flag: bool) -> Result<(), String> {
    let name = name.ok_or("시나리오 이름이 필요합니다. 예: cargo xtask try spike")?;
    let scenario = SCENARIOS.iter().find(|s| s.name == name).ok_or_else(|| {
        format!("알 수 없는 시나리오: {name}\n`cargo xtask help`로 목록을 확인하세요.")
    })?;
    let package = scenario.package;

    println!("▶ {}", scenario.description);
    let root = workspace_root();
    let status = Command::new(cargo())
        .current_dir(&root)
        .args(["build", "--release", "--package", package])
        .status()
        .map_err(|e| format!("cargo 실행 실패: {e}"))?;
    if !status.success() {
        return Err(format!("{package} 빌드가 실패했습니다 ({status})"));
    }

    let exe = root
        .join("target/release")
        .join(format!("{package}{}", env::consts::EXE_SUFFIX));
    let work_args = prepare_work_files(&root, scenario)?;
    let mut args: Vec<&str> = scenario.args.to_vec();
    args.extend(work_args.iter().map(String::as_str));
    if scenario.wait {
        let mut command = Command::new(&exe);
        command.current_dir(&root).args(&args);
        let status = command
            .status()
            .map_err(|e| format!("{} 실행 실패: {e}", exe.display()))?;
        if !status.success() {
            return Err(format!("{package} 실행이 실패했습니다 ({status})"));
        }
    } else {
        spawn_gui(&exe, &args, &root, wait_flag)?;
    }
    Ok(())
}

/// 시나리오의 `work_files`를 `target/try-<이름>/`에 새로 복사하고 복사본의 절대 경로를 돌려준다.
fn prepare_work_files(root: &Path, scenario: &Scenario) -> Result<Vec<String>, String> {
    if scenario.work_files.is_empty() {
        return Ok(Vec::new());
    }
    let dir = root.join("target").join(format!("try-{}", scenario.name));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("{} 정리 실패: {e}", dir.display()))?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{} 생성 실패: {e}", dir.display()))?;
    let mut paths = Vec::new();
    for file in scenario.work_files {
        let src = root.join(file);
        let name = src
            .file_name()
            .ok_or_else(|| format!("파일 이름 없음: {file}"))?;
        let dst = dir.join(name);
        if src.is_dir() {
            copy_dir(&src, &dst).map_err(|e| format!("{file} 복사 실패: {e}"))?;
            continue;
        }
        std::fs::copy(&src, &dst).map_err(|e| format!("{file} 복사 실패: {e}"))?;
        paths.push(dst.display().to_string());
    }
    println!("  작업 사본: {} ({}개)", dir.display(), paths.len());
    Ok(paths)
}

/// 폴더를 하위 폴더까지 복사한다.
fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &to)?;
        } else {
            std::fs::copy(entry.path(), to)?;
        }
    }
    Ok(())
}

/// GUI 앱을 실행한다.
/// Windows에서는 에이전트의 격리 데스크톱(`WinSta0\exebox-...`)에서 호출되더라도
/// 실제 사용자 화면(`WinSta0\Default`)에 창이 뜨도록 `STARTUPINFOW.lpDesktop`을 명시한다.
#[cfg(windows)]
fn spawn_gui(exe: &Path, args: &[&str], current_dir: &Path, wait_flag: bool) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    #[repr(C)]
    struct StartupInfoW {
        cb: u32,
        lp_reserved: *mut u16,
        lp_desktop: *mut u16,
        lp_title: *mut u16,
        dw_x: u32,
        dw_y: u32,
        dw_x_size: u32,
        dw_y_size: u32,
        dw_x_count_chars: u32,
        dw_y_count_chars: u32,
        dw_fill_attribute: u32,
        dw_flags: u32,
        w_show_window: u16,
        cb_reserved2: u16,
        lp_reserved2: *mut u8,
        h_std_input: usize,
        h_std_output: usize,
        h_std_error: usize,
    }

    #[repr(C)]
    struct ProcessInformation {
        h_process: usize,
        h_thread: usize,
        dw_process_id: u32,
        dw_thread_id: u32,
    }

    unsafe extern "system" {
        fn CreateProcessW(
            lp_application_name: *const u16,
            lp_command_line: *mut u16,
            lp_process_attributes: *const core::ffi::c_void,
            lp_thread_attributes: *const core::ffi::c_void,
            b_inherit_handles: i32,
            dw_creation_flags: u32,
            lp_environment: *const core::ffi::c_void,
            lp_current_directory: *const u16,
            lp_startup_info: *const StartupInfoW,
            lp_process_information: *mut ProcessInformation,
        ) -> i32;
        fn WaitForSingleObject(h_handle: usize, dw_milliseconds: u32) -> u32;
        fn CloseHandle(h_object: usize) -> i32;
        fn GetLastError() -> u32;
    }

    fn to_wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(std::iter::once(0)).collect()
    }

    let mut cmd_str = format!("\"{}\"", exe.display());
    for arg in args {
        cmd_str.push(' ');
        if arg.contains(' ') {
            cmd_str.push('"');
            cmd_str.push_str(arg);
            cmd_str.push('"');
        } else {
            cmd_str.push_str(arg);
        }
    }

    let app_wide = to_wide(exe.as_os_str());
    let mut cmd_wide = to_wide(OsStr::new(&cmd_str));
    let dir_wide = to_wide(current_dir.as_os_str());
    let mut desktop_wide = to_wide(OsStr::new(r"WinSta0\Default"));

    let si = StartupInfoW {
        cb: std::mem::size_of::<StartupInfoW>() as u32,
        lp_reserved: ptr::null_mut(),
        lp_desktop: desktop_wide.as_mut_ptr(),
        lp_title: ptr::null_mut(),
        dw_x: 0,
        dw_y: 0,
        dw_x_size: 0,
        dw_y_size: 0,
        dw_x_count_chars: 0,
        dw_y_count_chars: 0,
        dw_fill_attribute: 0,
        dw_flags: 0x0000_0001, // STARTF_USESHOWWINDOW
        w_show_window: 1,      // SW_SHOWNORMAL
        cb_reserved2: 0,
        lp_reserved2: ptr::null_mut(),
        h_std_input: 0,
        h_std_output: 0,
        h_std_error: 0,
    };
    let mut pi = ProcessInformation {
        h_process: 0,
        h_thread: 0,
        dw_process_id: 0,
        dw_thread_id: 0,
    };

    // SAFETY: 모든 포인터는 널 종료 UTF-16 버퍼를 가리키며 호출 동안 유효하다.
    let ok = unsafe {
        CreateProcessW(
            app_wide.as_ptr(),
            cmd_wide.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            0,
            ptr::null(),
            dir_wide.as_ptr(),
            &si,
            &mut pi,
        )
    };
    if ok == 0 {
        let err = unsafe { GetLastError() };
        return Err(format!("{} 실행 실패 (Win32 error {err})", exe.display()));
    }

    println!(
        "  실행됨 (pid {}, WinSta0\\Default) — 창을 닫으면 종료됩니다",
        pi.dw_process_id
    );
    unsafe {
        CloseHandle(pi.h_thread);
        if wait_flag {
            WaitForSingleObject(pi.h_process, 0xFFFF_FFFF);
        }
        CloseHandle(pi.h_process);
    }
    Ok(())
}

#[cfg(not(windows))]
fn spawn_gui(exe: &Path, args: &[&str], current_dir: &Path, wait_flag: bool) -> Result<(), String> {
    use std::process::Stdio;
    let mut child = Command::new(exe)
        .current_dir(current_dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("{} 실행 실패: {e}", exe.display()))?;
    println!("  실행됨 (pid {}) — 창을 닫으면 종료됩니다", child.id());
    if wait_flag {
        let _ = child.wait();
    }
    Ok(())
}

fn doctor() -> Result<(), String> {
    let root = workspace_root();
    let mut warnings = 0;

    println!("OS          : {} ({})", env::consts::OS, env::consts::ARCH);
    println!("rustc       : {}", output("rustc", &["--version"], &root));
    println!("cargo       : {}", output(&cargo(), &["--version"], &root));
    println!("git         : {}", output("git", &["--version"], &root));
    println!(
        "git 사용자   : {} <{}>",
        output("git", &["config", "user.name"], &root),
        output("git", &["config", "user.email"], &root)
    );
    println!(
        "원격 origin : {}",
        output("git", &["remote", "get-url", "origin"], &root)
    );
    println!(
        "현재 브랜치 : {}",
        output("git", &["branch", "--show-current"], &root)
    );

    let autocrlf = output("git", &["config", "core.autocrlf"], &root);
    if autocrlf == "true" {
        warnings += 1;
        println!(
            "⚠ core.autocrlf=true — 이 저장소에서는 false여야 합니다: git config core.autocrlf false"
        );
    } else {
        println!("core.autocrlf: {autocrlf} (정상)");
    }

    // 픽스처가 체크아웃 과정에서 변형되지 않았는지 확인한다 (§0.8 줄바꿈 규칙).
    let crlf_fixture = root.join("tests/fixtures/line_ending/crlf.txt");
    match std::fs::read(&crlf_fixture) {
        Ok(bytes) if bytes.windows(2).any(|w| w == b"\r\n") => {
            println!("CRLF 픽스처 : 바이트 보존됨 (정상)");
        }
        Ok(_) => {
            warnings += 1;
            println!(
                "⚠ CRLF 픽스처의 CRLF가 사라졌습니다 — .gitattributes 또는 git 설정을 확인하세요"
            );
        }
        Err(_) => {
            warnings += 1;
            println!(
                "⚠ CRLF 픽스처를 찾을 수 없습니다: {}",
                crlf_fixture.display()
            );
        }
    }

    if warnings == 0 {
        println!("\n✔ 개발 환경 점검 통과");
        Ok(())
    } else {
        Err(format!("경고 {warnings}건"))
    }
}

fn cargo() -> String {
    env::var("CARGO").unwrap_or_else(|_| "cargo".to_string())
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask는 워크스페이스 루트 아래에 있어야 합니다")
        .to_path_buf()
}

/// 명령을 실행해 표준 출력 첫 줄을 돌려준다. 실패하면 "(없음)".
fn output(program: &str, args: &[&str], dir: &Path) -> String {
    Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.lines().next().map(str::to_string))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(없음)".to_string())
}
