//! P0-07 메모리·기동 시간 측정용 실행 인자.
//!
//! - `--renderer=glow|wgpu`: 렌더링 백엔드 선택 (기본 wgpu)
//! - `--no-ko-fonts`: 한글 폰트를 읽지 않고 egui 기본 폰트만 사용 (기준선)
//! - `--open <파일>`: 성능 화면에서 파일을 열어 둔 상태로 시작
//! - `--measure-out <파일>`: 첫 프레임이 그려진 뒤 `first_frame_ms=<값>`을 파일에 쓴다
//!
//! `cargo xtask measure`가 이 인자로 앱을 여러 번 실행해 평균을 낸다.

use std::path::PathBuf;
use std::time::Instant;

#[derive(Default)]
pub struct Args {
    pub renderer: Option<String>,
    pub no_ko_fonts: bool,
    pub open: Option<PathBuf>,
    pub measure_out: Option<PathBuf>,
}

impl Args {
    pub fn parse() -> Self {
        let mut args = Self::default();
        let mut iter = std::env::args().skip(1);
        while let Some(arg) = iter.next() {
            if let Some(r) = arg.strip_prefix("--renderer=") {
                args.renderer = Some(r.to_owned());
            } else if arg == "--no-ko-fonts" {
                args.no_ko_fonts = true;
            } else if arg == "--open" {
                args.open = iter.next().map(PathBuf::from);
            } else if arg == "--measure-out" {
                args.measure_out = iter.next().map(PathBuf::from);
            }
        }
        args
    }

    pub fn renderer(&self) -> eframe::Renderer {
        match self.renderer.as_deref() {
            Some("glow") => eframe::Renderer::Glow,
            _ => eframe::Renderer::Wgpu,
        }
    }

    /// 두 번째 프레임 시작 시점(= 첫 프레임이 화면에 나간 뒤)에 한 번 호출한다.
    pub fn report_first_frame(&self, started: Instant) {
        if let Some(path) = &self.measure_out {
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            let _ = std::fs::write(path, format!("first_frame_ms={ms:.1}\n"));
        }
    }
}
