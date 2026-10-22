//! My-Notepad 엔트리 포인트 (PLAN.md §4.2 · WORKPLAN.md P1-01, P1-02).
//!
//! - 로깅 및 panic hook 초기화 (`logging.rs`)
//! - `--test-panic`: panic hook이 로그 파일에 정보를 남기는지 검증하기 위한 플래그
//! - `--font-report`: 창을 띄우지 않고 OS 폰트 Fallback 체인 로드 보고서를 출력 (자동 검증용)
//! - `--log-file <경로>`: 로그 파일 경로 수동 지정 (자동 테스트용)
//! - 그 밖의 인자는 열 파일 경로 — 탭으로 연다 (`cli.rs`, P1-13: 상대 경로·`--`·탐색기 연결 프로그램)
//! - `eframe` `glow` 렌더러로 네이티브 창 기동

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use my_notepad::{app, cli::CliArgs, fonts, logging};

fn main() -> eframe::Result {
    let cli = CliArgs::from_env();
    let log_path = logging::init(cli.log_file.clone());
    log::info!(
        "My-Notepad v{} 기동 ({}/{})",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    if let Some(path) = &log_path {
        log::info!("로그 파일: {}", path.display());
    }
    for flag in &cli.unknown {
        log::warn!("알 수 없는 명령줄 플래그 무시: {flag}");
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
            .with_min_inner_size([480.0, 320.0])
            // 탐색기에서 파일을 끌어다 놓아 열기 (P1-13)
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "my-notepad",
        options,
        Box::new(move |cc| Ok(Box::new(app::NotepadApp::new(cc, &cli.files)))),
    )
}
