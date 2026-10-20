//! Windows·macOS 공통 개발 보조 명령.
//!
//! 사용법: `cargo xtask <명령> [인자]` (별칭은 `.cargo/config.toml`)

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

/// `cargo xtask try <시나리오>`로 실행할 수 있는 시나리오.
struct Scenario {
    name: &'static str,
    package: &'static str,
    args: &'static [&'static str],
    description: &'static str,
    /// true면 끝날 때까지 기다려 출력을 보여준다. false면 창을 띄우고 바로 돌아온다
    /// (xtask가 앱을 붙잡고 있으면 다음 빌드에서 xtask.exe를 교체하지 못한다).
    wait: bool,
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "spike",
        package: "editor-spike",
        args: &[],
        description: "에디터 스파이크 (입력 검증 + 폰트 화면)",
        wait: false,
    },
    Scenario {
        name: "input",
        package: "editor-spike",
        args: &[],
        description: "P0-03~05 한글 IME·단축키·창 닫기 검증 화면",
        wait: false,
    },
    Scenario {
        name: "fonts",
        package: "editor-spike",
        args: &[],
        description: "P0-02 폰트 Fallback 체인 확인 화면",
        wait: false,
    },
    Scenario {
        name: "prototype",
        package: "ux-prototype",
        args: &[],
        description: "P0-UX1 전체 화면 UX 프로토타입 (가짜 데이터)",
        wait: false,
    },
    Scenario {
        name: "font-report",
        package: "editor-spike",
        args: &["--font-report"],
        description: "P0-02 폰트 로드 보고서만 출력 (창 없음)",
        wait: true,
    },
];

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("try") => try_scenario(args.get(1).map(String::as_str)),
        Some("doctor") => doctor(),
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
    println!("  try <시나리오>   릴리스 빌드 후 실행 (사용자 검증용)");
    println!("  doctor           개발 환경 점검 (두 장비 준비 상태 확인)");
    println!("  help             이 도움말\n");
    println!("시나리오:");
    for scenario in SCENARIOS {
        println!("  {:<16} {}", scenario.name, scenario.description);
    }
}

fn try_scenario(name: Option<&str>) -> Result<(), String> {
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
    let mut command = Command::new(&exe);
    command.current_dir(&root).args(scenario.args);
    if scenario.wait {
        let status = command
            .status()
            .map_err(|e| format!("{} 실행 실패: {e}", exe.display()))?;
        if !status.success() {
            return Err(format!("{package} 실행이 실패했습니다 ({status})"));
        }
    } else {
        // 출력을 떼어 내야 `cargo xtask try ... | tail` 같은 파이프가 앱 종료까지 기다리지 않는다.
        let child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("{} 실행 실패: {e}", exe.display()))?;
        println!("  실행됨 (pid {}) — 창을 닫으면 종료됩니다", child.id());
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
