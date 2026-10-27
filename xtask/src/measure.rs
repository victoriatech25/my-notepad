//! `cargo xtask measure [--runs N]`: 메모리·기동 시간 측정 (P0-07).
//!
//! editor-spike를 (렌더러 × 한글 폰트 × 문서) 조합별로 여러 번 실행해
//! 첫 프레임까지 걸린 시간과 안정된 뒤의 메모리(Windows Working Set / macOS RSS)를 평균 낸다.
//!
//! `--viewer` (P1-14): 본 앱(`my-notepad`)으로 10MB·50MB 샘플을 열어 읽기 전용 뷰어의 첫 화면 시간,
//! 문서 전체를 훑는 동안의 프레임 CPU 시간, 메모리를 재고 PLAN.md §1.3 목표와 비교한다.

use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

const SETTLE: Duration = Duration::from_millis(1500);
const TIMEOUT: Duration = Duration::from_secs(120);

struct Case {
    renderer: &'static str,
    ko_fonts: bool,
    doc: Option<&'static str>,
}

pub fn run(root: &Path, runs: usize) -> Result<(), String> {
    let status = Command::new(super::cargo())
        .current_dir(root)
        .args(["build", "--release", "--package", "editor-spike"])
        .status()
        .map_err(|e| format!("cargo 실행 실패: {e}"))?;
    if !status.success() {
        return Err("editor-spike 빌드 실패".into());
    }
    let exe = root
        .join("target/release")
        .join(format!("editor-spike{}", std::env::consts::EXE_SUFFIX));
    let sample = root.join("spikes/samples/text-10mb.txt");
    if !sample.exists() {
        return Err("샘플이 없습니다. 먼저 `cargo xtask gen-samples`를 실행하세요.".into());
    }
    let size = std::fs::metadata(&exe).map(|m| m.len()).unwrap_or(0);
    println!(
        "실행 파일: {} ({:.2} MB, glow+wgpu 포함)\n",
        exe.display(),
        size as f64 / 1_048_576.0
    );

    let mut cases = Vec::new();
    for renderer in ["wgpu", "glow"] {
        for ko_fonts in [true, false] {
            for doc in [None, Some("text-10mb.txt")] {
                cases.push(Case {
                    renderer,
                    ko_fonts,
                    doc,
                });
            }
        }
    }

    println!(
        "{:<6} {:<10} {:<12} {:>5} {:>14} {:>14}",
        "렌더러", "한글 폰트", "문서", "횟수", "첫 프레임 ms", "메모리 MB"
    );
    for case in &cases {
        // 10MB 문서는 첫 레이아웃만 수 초가 걸리므로 횟수를 줄인다.
        let n = if case.doc.is_some() {
            runs.min(3)
        } else {
            runs
        };
        let mut first_frames = Vec::new();
        let mut memories = Vec::new();
        for i in 0..n {
            let out = std::env::temp_dir()
                .join(format!("my-notepad-measure-{}-{i}.txt", std::process::id()));
            let _ = std::fs::remove_file(&out);
            let mut command = Command::new(&exe);
            command
                .current_dir(root)
                .arg(format!("--renderer={}", case.renderer))
                .arg("--measure-out")
                .arg(&out)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if !case.ko_fonts {
                command.arg("--no-ko-fonts");
            }
            if let Some(doc) = case.doc {
                command
                    .arg("--open")
                    .arg(root.join("spikes/samples").join(doc));
            }
            let mut child = command.spawn().map_err(|e| format!("실행 실패: {e}"))?;

            let started = Instant::now();
            let first_frame = loop {
                if let Ok(text) = std::fs::read_to_string(&out)
                    && let Some(v) = text.trim().strip_prefix("first_frame_ms=")
                {
                    break v.parse::<f64>().ok();
                }
                if started.elapsed() > TIMEOUT {
                    break None;
                }
                sleep(Duration::from_millis(50));
            };
            sleep(SETTLE);
            let memory = memory_mb(child.id());
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&out);
            if let Some(ms) = first_frame {
                first_frames.push(ms);
            }
            if let Some(mb) = memory {
                memories.push(mb);
            }
        }
        println!(
            "{:<6} {:<10} {:<12} {:>5} {:>14} {:>14}",
            case.renderer,
            if case.ko_fonts {
                "직접 구성"
            } else {
                "없음"
            },
            case.doc.unwrap_or("빈 문서"),
            n,
            stats(&first_frames),
            stats(&memories),
        );
    }
    println!("\n메모리: Windows는 Working Set, macOS는 RSS. 첫 프레임 표시 {SETTLE:?} 뒤에 측정.");
    Ok(())
}

struct ViewerCase {
    sample: &'static str,
    first_frame_target_ms: f64,
    memory_target_mb: Option<f64>,
}

/// 본 앱 대용량 뷰어 측정 (`cargo xtask measure --viewer`).
pub fn run_viewer(root: &Path, runs: usize) -> Result<(), String> {
    let status = Command::new(super::cargo())
        .current_dir(root)
        .args(["build", "--release", "--package", "my-notepad"])
        .status()
        .map_err(|e| format!("cargo 실행 실패: {e}"))?;
    if !status.success() {
        return Err("my-notepad 빌드 실패".into());
    }
    let exe = root
        .join("target/release")
        .join(format!("my-notepad{}", std::env::consts::EXE_SUFFIX));
    let cases = [
        ViewerCase {
            sample: "text-10mb.txt",
            first_frame_target_ms: 1000.0,
            memory_target_mb: Some(150.0),
        },
        ViewerCase {
            sample: "text-50mb.txt",
            first_frame_target_ms: 2000.0,
            memory_target_mb: None,
        },
    ];
    for case in &cases {
        if !root.join("spikes/samples").join(case.sample).exists() {
            return Err("샘플이 없습니다. 먼저 `cargo xtask gen-samples`를 실행하세요.".into());
        }
    }

    println!(
        "{:<14} {:>4} {:>16} {:>18} {:>18} {:>14}",
        "문서", "횟수", "첫 화면 ms", "스크롤 평균 ms", "스크롤 최대 ms", "메모리 MB"
    );
    for case in &cases {
        let mut first = Vec::new();
        let mut avg = Vec::new();
        let mut max = Vec::new();
        let mut memory = Vec::new();
        for i in 0..runs {
            let out = std::env::temp_dir()
                .join(format!("my-notepad-viewer-{}-{i}.txt", std::process::id()));
            let _ = std::fs::remove_file(&out);
            let mut child = Command::new(&exe)
                .current_dir(root)
                .arg("--measure-out")
                .arg(&out)
                .arg(root.join("spikes/samples").join(case.sample))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("실행 실패: {e}"))?;
            let started = Instant::now();
            let report = loop {
                if let Ok(text) = std::fs::read_to_string(&out)
                    && text.contains("done=1")
                {
                    break Some(text);
                }
                if started.elapsed() > TIMEOUT {
                    break None;
                }
                sleep(Duration::from_millis(50));
            };
            sleep(SETTLE);
            let mb = memory_mb(child.id());
            let _ = child.kill();
            let _ = child.wait();
            let _ = std::fs::remove_file(&out);
            let Some(report) = report else {
                continue;
            };
            let value = |key: &str| -> Option<f64> {
                report
                    .lines()
                    .find_map(|l| l.strip_prefix(key)?.strip_prefix('=')?.parse().ok())
            };
            if !report.contains("viewer=true") {
                return Err(format!("{}이(가) 뷰어로 열리지 않았습니다", case.sample));
            }
            first.extend(value("first_frame_ms"));
            avg.extend(value("scroll_frame_ms_avg"));
            max.extend(value("scroll_frame_ms_max"));
            memory.extend(mb);
        }
        println!(
            "{:<14} {:>4} {:>16} {:>18} {:>18} {:>14}",
            case.sample,
            runs,
            stats(&first),
            stats(&avg),
            stats(&max),
            stats(&memory),
        );
        let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
        let judge = |ok: bool| if ok { "충족" } else { "미달" };
        println!(
            "  목표: 첫 화면 ≤ {:.0}ms {} · 스크롤 프레임 ≤ 16ms {}{}",
            case.first_frame_target_ms,
            judge(!first.is_empty() && mean(&first) <= case.first_frame_target_ms),
            judge(!avg.is_empty() && mean(&avg) <= 16.0),
            case.memory_target_mb.map_or(String::new(), |t| format!(
                " · 메모리 ≤ {t:.0}MB {}",
                judge(!memory.is_empty() && mean(&memory) <= t)
            )),
        );
    }
    println!(
        "
스크롤: 문서 처음부터 끝까지 {} 구간을 한 프레임씩 건너뛰며 eframe cpu_usage(앱 로직 + 그리기, vsync 대기 제외) 기록.",
        60
    );
    Ok(())
}

/// 본 앱 빈 문서 측정 (`cargo xtask measure --app`, P5-06): 첫 화면 시간·메모리를 PLAN.md §1.3 목표
/// (warm 기동 ≤ 400ms, 빈 문서 메모리 ≤ 120MB)와 비교한다. 측정 실행은 설정 파일을 읽지 않는다(기본 글꼴).
pub fn run_app(root: &Path, runs: usize) -> Result<(), String> {
    let status = Command::new(super::cargo())
        .current_dir(root)
        .args(["build", "--release", "--package", "my-notepad"])
        .status()
        .map_err(|e| format!("cargo 실행 실패: {e}"))?;
    if !status.success() {
        return Err("my-notepad 빌드 실패".into());
    }
    let exe = root
        .join("target/release")
        .join(format!("my-notepad{}", std::env::consts::EXE_SUFFIX));
    let size = std::fs::metadata(&exe).map(|m| m.len()).unwrap_or(0);
    let mut first = Vec::new();
    let mut memory = Vec::new();
    for i in 0..runs {
        let out =
            std::env::temp_dir().join(format!("my-notepad-app-{}-{i}.txt", std::process::id()));
        let _ = std::fs::remove_file(&out);
        let mut child = Command::new(&exe)
            .current_dir(root)
            .arg("--measure-out")
            .arg(&out)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("실행 실패: {e}"))?;
        let started = Instant::now();
        let report = loop {
            if let Ok(text) = std::fs::read_to_string(&out)
                && text.contains("done=1")
            {
                break Some(text);
            }
            if started.elapsed() > TIMEOUT {
                break None;
            }
            sleep(Duration::from_millis(50));
        };
        sleep(SETTLE);
        memory.extend(memory_mb(child.id()));
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&out);
        first.extend(report.and_then(|r| {
            r.lines()
                .find_map(|l| l.strip_prefix("first_frame_ms=")?.parse::<f64>().ok())
        }));
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    let judge = |ok: bool| if ok { "충족" } else { "미달" };
    println!(
        "본 앱 빈 문서 ({runs}회, 실행 파일 {:.2}MB)\n  첫 화면 ms: {}\n  메모리 MB : {}",
        size as f64 / 1_048_576.0,
        stats(&first),
        stats(&memory)
    );
    println!(
        "  목표: warm 기동 ≤ 400ms {} · 메모리 ≤ 120MB {} · 실행 파일 ≤ 15MB {}",
        judge(!first.is_empty() && mean(&first) <= 400.0),
        judge(!memory.is_empty() && mean(&memory) <= 120.0),
        judge(size as f64 <= 15.0 * 1_048_576.0)
    );
    Ok(())
}

/// "평균 (최소~최대)"
fn stats(values: &[f64]) -> String {
    if values.is_empty() {
        return "실패".to_owned();
    }
    let avg = values.iter().sum::<f64>() / values.len() as f64;
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(0.0, f64::max);
    format!("{avg:.0} ({min:.0}~{max:.0})")
}

fn memory_mb(pid: u32) -> Option<f64> {
    if cfg!(windows) {
        let output = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("(Get-Process -Id {pid}).WorkingSet64"),
            ])
            .output()
            .ok()?;
        let bytes: f64 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .ok()?;
        Some(bytes / 1_048_576.0)
    } else {
        let output = Command::new("ps")
            .args(["-o", "rss=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let kb: f64 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .ok()?;
        Some(kb / 1024.0)
    }
}
