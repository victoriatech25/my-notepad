//! Phase 0 에디터 스파이크.
//!
//! - P0-01: 버전을 고정한 eframe/egui로 창을 띄우고 텍스트 입력란 하나를 표시
//! - P0-02: OS 시스템 폰트로 Monospace / Proportional / 굵게 Fallback 체인 구성 (`fonts.rs`)
//! - P0-03~05: 한글 IME 이벤트 로그, 위젯보다 먼저 처리하는 단축키, 창 닫기 가로채기 (`input_check.rs`)
//!
//! `--font-report`: 창을 띄우지 않고 폰트 로드 보고서만 출력한다 (자동 검증용).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod fonts;
mod input_check;
mod measure;
mod perf;

use std::time::{Duration, Instant};

use eframe::egui::{self, FontFamily, RichText};

const SAMPLE_TEXT: &str = "\
English: The quick brown fox jumps over the lazy dog. 0123456789
한글: 다람쥐 헌 쳇바퀴에 타고파. 키스의 고유조건은 입술끼리 만나야 하고 특별한 기술은 필요치 않다.
기호: ()[]{}<>!@#$%^&*-_=+|\\/:;\"'`~ ← → ★ ※ ①②③

[정렬 확인: 영문·숫자 줄]
ID    NAME        QTY    PRICE
0001  apple         12    1,200
0002  banana       345   34,500
0003  cherry      6789  678,900

[정렬 확인: 한글 포함 줄 — 참고용]
ID    이름        수량   가격
0004  사과          12    1,200
0005  바나나       345   34,500

Tab 키 입력 확인 →\t← 이 사이에 탭 문자가 있습니다. 여기서도 Tab을 눌러 보세요:
";

const PROPORTIONAL_SAMPLE: &str = "메뉴·대화상자·Markdown 미리보기 본문에 쓰이는 비례폭 글꼴입니다. \
한글과 English가 섞인 문장도 자연스럽게 이어져야 합니다. 숫자 1234567890과 기호 (괄호), \"따옴표\"도 확인합니다.";

fn main() -> eframe::Result {
    let started = Instant::now();

    if std::env::args().any(|a| a == "--perf-bench") {
        let (defs, _) = fonts::explicit_chain();
        print!("{}", perf::bench(defs));
        std::process::exit(0);
    }

    if std::env::args().any(|a| a == "--font-report") {
        let (_, report) = fonts::explicit_chain();
        print!("{}", report.to_text());
        std::process::exit(if report.failures.is_empty() { 0 } else { 1 });
    }

    let args = measure::Args::parse();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("My-Notepad Spike (Phase 0)")
            .with_inner_size([1200.0, 780.0])
            .with_min_inner_size([480.0, 320.0]),
        renderer: args.renderer(),
        // 한글 폰트 없이 잴 때는 시스템 폰트 자동 대체도 꺼서 순수 기준선을 만든다.
        system_font_fallback: !args.no_ko_fonts,
        ..Default::default()
    };
    eframe::run_native(
        "my-notepad-editor-spike",
        options,
        Box::new(move |cc| Ok(Box::new(SpikeApp::new(cc, started, args)))),
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    /// P0-03~05: IME·단축키·창 닫기
    Input,
    /// P0-02: 폰트 Fallback 체인
    Fonts,
    /// P0-06: 대용량 텍스트 성능
    Perf,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FontMode {
    /// PLAN.md §2.4 방식: 시스템 폰트를 직접 체인으로 구성 (P0-02에서 채택)
    ExplicitChain,
    /// 비교용: egui 기본 폰트 + eframe 시스템 폰트 자동 대체
    EguiDefault,
}

struct SpikeApp {
    screen: Screen,
    text: String,
    font_mode: FontMode,
    explicit_defs: egui::FontDefinitions,
    report: fonts::FontReport,
    input: input_check::InputCheck,
    perf: perf::PerfScreen,
    started: Instant,
    first_frame: Option<Duration>,
    args: measure::Args,
    frames: u64,
}

impl SpikeApp {
    fn new(cc: &eframe::CreationContext<'_>, started: Instant, args: measure::Args) -> Self {
        let (explicit_defs, report) = if args.no_ko_fonts {
            (
                egui::FontDefinitions::default(),
                fonts::FontReport::default(),
            )
        } else {
            fonts::explicit_chain()
        };
        cc.egui_ctx.set_fonts(explicit_defs.clone());
        let mut perf = perf::PerfScreen::default();
        if let Some(path) = &args.open {
            perf.load(path.clone());
        }
        Self {
            screen: if args.open.is_some() || std::env::args().any(|a| a == "--screen=perf") {
                Screen::Perf
            } else {
                Screen::Input
            },
            text: SAMPLE_TEXT.to_owned(),
            font_mode: FontMode::ExplicitChain,
            explicit_defs,
            report,
            input: input_check::InputCheck::default(),
            perf,
            started,
            first_frame: None,
            args,
            frames: 0,
        }
    }

    fn apply_font_mode(&self, ctx: &egui::Context) {
        match self.font_mode {
            FontMode::ExplicitChain => ctx.set_fonts(self.explicit_defs.clone()),
            FontMode::EguiDefault => ctx.set_fonts(fonts::egui_default()),
        }
    }

    fn fonts_screen(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("font-mode").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                let before = self.font_mode;
                ui.label("글꼴 방식:");
                ui.radio_value(
                    &mut self.font_mode,
                    FontMode::ExplicitChain,
                    "직접 구성 (채택)",
                );
                ui.radio_value(
                    &mut self.font_mode,
                    FontMode::EguiDefault,
                    "egui 기본 + 시스템 자동 대체 (비교용)",
                );
                if self.font_mode != before {
                    self.apply_font_mode(ui.ctx());
                }
            });
        });

        egui::Panel::left("report")
            .default_size(360.0)
            .show(ui, |ui| {
                ui.heading("폰트 로드 보고서");
                ui.label("직접 구성 방식에서 읽은 시스템 폰트 (memory-map)");
                ui.add_space(4.0);
                ui.label(RichText::new(self.report.to_text()).monospace().small());
                if !self.report.failures.is_empty() {
                    ui.colored_label(
                        ui.visuals().error_fg_color,
                        "일부 폰트를 찾지 못해 egui 기본 폰트로 대체했습니다.",
                    );
                }
                ui.add_space(8.0);
                ui.label("체인 순서");
                ui.label(
                    RichText::new(
                        "Monospace    = mono → ko → egui 기본\n\
                     Proportional = ui → ko → egui 기본\n\
                     bold         = ui-bold → ko-bold → egui 기본",
                    )
                    .monospace()
                    .small(),
                );
            });

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                ui.heading("1. 에디터 (Monospace)");
                ui.add(
                    egui::TextEdit::multiline(&mut self.text)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .desired_rows(18)
                        // Tab 키가 포커스를 옮기지 않고 탭 문자로 입력되게 한다.
                        .lock_focus(true),
                );

                ui.add_space(12.0);
                ui.heading("2. 비례폭 (Proportional)");
                ui.label(PROPORTIONAL_SAMPLE);

                ui.add_space(12.0);
                ui.heading("3. 굵게 (bold 패밀리)");
                ui.horizontal(|ui| {
                    ui.label(RichText::new("보통 한글 Regular 123").size(20.0));
                    ui.separator();
                    ui.label(
                        RichText::new("굵은 한글 Bold 123")
                            .size(20.0)
                            .family(FontFamily::Name(fonts::BOLD_FAMILY.into())),
                    );
                });
            });
        });
    }
}

impl eframe::App for SpikeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frames += 1;
        if self.frames == 2 {
            self.args.report_first_frame(self.started);
        }
        if self.first_frame.is_none() {
            self.first_frame = Some(self.started.elapsed());
        }
        let ctx = ui.ctx().clone();
        let pixels_per_point = ctx.pixels_per_point();

        // 입력 이벤트 기록과 단축키 선처리는 어떤 위젯보다 먼저 한다 (P0-04).
        self.input.before_widgets(&ctx);

        egui::Panel::top("info").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.strong("Phase 0 스파이크");
                ui.separator();
                ui.selectable_value(&mut self.screen, Screen::Input, "입력 검증 (P0-03~05)");
                ui.selectable_value(&mut self.screen, Screen::Fonts, "폰트 (P0-02)");
                ui.selectable_value(&mut self.screen, Screen::Perf, "대용량 성능 (P0-06)");
                ui.separator();
                ui.label(format!(
                    "egui {} · {} ({})",
                    env!("EGUI_VERSION"),
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ));
            });
        });

        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(format!("scale {pixels_per_point:.2}"));
                if let Some(first_frame) = self.first_frame {
                    ui.separator();
                    ui.label(format!(
                        "첫 프레임 {:.0} ms",
                        first_frame.as_secs_f64() * 1000.0
                    ));
                }
                if self.screen == Screen::Input {
                    ui.separator();
                    self.input.status(ui);
                }
            });
        });

        match self.screen {
            Screen::Input => self.input.screen(ui),
            Screen::Fonts => self.fonts_screen(ui),
            Screen::Perf => self.perf.screen(ui),
        }

        self.input.close_dialog(&ctx);
    }
}
