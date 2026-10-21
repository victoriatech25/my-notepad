//! My-Notepad 엔트리 포인트 (PLAN.md §4.2 · WORKPLAN.md P1-01, P1-02).
//!
//! - 로깅 및 panic hook 초기화 (`logging.rs`)
//! - `--test-panic`: panic hook이 로그 파일에 정보를 남기는지 검증하기 위한 플래그
//! - `--font-report`: 창을 띄우지 않고 OS 폰트 Fallback 체인 로드 보고서를 출력 (자동 검증용)
//! - `--log-file <경로>`: 로그 파일 경로 수동 지정 (자동 테스트용)
//! - `eframe` `glow` 렌더러로 네이티브 창 기동

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;

use eframe::egui;
use my_notepad::{app, fonts, logging};

struct CliArgs {
    log_file: Option<PathBuf>,
    test_panic: bool,
    font_report: bool,
}

impl CliArgs {
    fn parse() -> Self {
        let mut args = std::env::args().skip(1);
        let mut log_file = None;
        let mut test_panic = false;
        let mut font_report = false;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--test-panic" => test_panic = true,
                "--font-report" => font_report = true,
                "--log-file" => {
                    if let Some(p) = args.next() {
                        log_file = Some(PathBuf::from(p));
                    }
                }
                _ => {
                    if let Some(p) = arg.strip_prefix("--log-file=") {
                        log_file = Some(PathBuf::from(p));
                    }
                }
            }
        }

        Self {
            log_file,
            test_panic,
            font_report,
        }
    }
}

fn main() -> eframe::Result {
    let cli = CliArgs::parse();
    let log_path = logging::init(cli.log_file);
    log::info!(
        "My-Notepad v{} 기동 ({}/{})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    if let Some(path) = &log_path {
        log::info!("로그 파일: {}", path.display());
    }

    if cli.test_panic {
        panic!("P1-01 의도적 테스트 panic 발생");
    }

    if cli.font_report {
        let (_, report) = fonts::explicit_chain();
        print!("{}", report.to_text());
        std::process::exit(if report.has_failures() { 1 } else { 0 });
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("My-Notepad")
            .with_inner_size([1200.0, 780.0])
            .with_min_inner_size([480.0, 320.0]),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "my-notepad",
        options,
        Box::new(|cc| Ok(Box::new(app::NotepadApp::new(cc)))),
    )
}
