//! My-Notepad 엔트리 포인트 (PLAN.md §4.2 · WORKPLAN.md P1-01, P1-02).
//!
//! - 로깅 및 panic hook 초기화 (`logging.rs`)
//! - `--test-panic`: panic hook이 로그 파일에 정보를 남기는지 검증하기 위한 플래그
//! - `--font-report`: 창을 띄우지 않고 OS 폰트 Fallback 체인 로드 보고서를 출력 (자동 검증용)
//! - `--log-file <경로>`: 로그 파일 경로 수동 지정 (자동 테스트용)
//! - 그 밖의 인자는 열 파일 경로 — 탭으로 연다 (`cli.rs`, P1-13: 상대 경로·`--`·탐색기 연결 프로그램)
//! - 이미 실행 중인 창이 있으면 파일을 넘기고 끝낸다 (`single_instance.rs`, P5-04). `--new-window`·측정 실행은 건너뛴다
//! - 저장된 설정(`config.rs`, P2-08)을 읽어 창 위치·크기를 되살린다. 성능 측정(`--measure-out`) 실행은 설정을 쓰지 않는다
//! - `eframe` `glow` 렌더러로 네이티브 창 기동

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use my_notepad::single_instance::{self, Listener, Startup};
use my_notepad::{app, branding, cli::CliArgs, config::Settings, fonts, logging};

fn main() -> eframe::Result {
    let started = std::time::Instant::now();
    let cli = CliArgs::from_env();
    let log_path = logging::init(cli.log_file.clone());
    log::info!(
        "{} v{} 기동 ({}/{})",
        branding::APP_NAME,
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

    // 측정 실행은 다른 창과 섞이면 안 되므로 늘 새 창으로 뜬다.
    let instance = if cli.new_window || cli.measure_out.is_some() {
        Listener::none()
    } else {
        match single_instance::acquire(&cli.files) {
            Startup::Primary(listener) => listener,
            Startup::Forwarded => return Ok(()),
        }
    };

    // 성능 측정은 저장된 창 크기·사이드바 폴더 등에 영향을 받지 않게 설정 없이 실행한다.
    let settings_path = if cli.measure_out.is_some() {
        None
    } else {
        Settings::default_path()
    };
    let (settings, settings_problem) = match &settings_path {
        Some(path) => {
            let loaded = Settings::load(path);
            (loaded.settings, loaded.problem)
        }
        None => (Settings::default(), None),
    };

    let mut viewport = egui::ViewportBuilder::default()
        .with_title(branding::APP_NAME)
        .with_inner_size([1200.0, 780.0])
        .with_min_inner_size([480.0, 320.0])
        // 탐색기에서 파일을 끌어다 놓아 열기 (P1-13)
        .with_drag_and_drop(true);
    if let Some(icon) = branding::window_icon() {
        viewport = viewport.with_icon(icon);
    }
    if let Some(window) = settings.window {
        viewport = viewport
            .with_inner_size([window.width, window.height])
            .with_position([window.x, window.y])
            .with_maximized(window.maximized);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "my-notepad",
        options,
        Box::new(move |cc| {
            let mut app =
                app::NotepadApp::new(cc, &cli.files, settings, settings_path, settings_problem);
            instance.attach(cc);
            app.set_instance_listener(instance);
            if let Some(out) = cli.measure_out.clone() {
                app.enable_measure(out, started);
            }
            Ok(Box::new(app))
        }),
    )
}
