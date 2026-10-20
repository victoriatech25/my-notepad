//! Windows·macOS 공통 개발 보조 명령.
//!
//! 사용법: `cargo xtask <명령> [인자]` (별칭은 `.cargo/config.toml`)

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// `cargo xtask try <시나리오>`로 실행할 수 있는 시나리오.
struct Scenario {
    name: &'static str,
    package: &'static str,
    args: &'static [&'static str],
    description: &'static str,
}

const SCENARIOS: &[Scenario] = &[
    Scenario {
        name: "spike",
        package: "editor-spike",
        args: &[],
        description: "에디터 스파이크 (현재 P0-02 상태)",
    },
    Scenario {
        name: "fonts",
        package: "editor-spike",
        args: &[],
        description: "P0-02 폰트 Fallback 체인 확인 화면",
    },
    Scenario {
        name: "font-report",
        package: "editor-spike",
        args: &["--font-report"],
        description: "P0-02 폰트 로드 보고서만 출력 (창 없음)",
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
    let status = Command::new(cargo())
        .current_dir(workspace_root())
        .args(["run", "--release", "--package", package, "--"])
        .args(scenario.args)
        .status()
        .map_err(|e| format!("cargo 실행 실패: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{package} 실행이 실패했습니다 ({status})"))
    }
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
