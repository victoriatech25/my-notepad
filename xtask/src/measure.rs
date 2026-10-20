//! `cargo xtask measure [--runs N]`: 메모리·기동 시간 측정 (P0-07).
//!
//! editor-spike를 (렌더러 × 한글 폰트 × 문서) 조합별로 여러 번 실행해
//! 첫 프레임까지 걸린 시간과 안정된 뒤의 메모리(Windows Working Set / macOS RSS)를 평균 낸다.

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
