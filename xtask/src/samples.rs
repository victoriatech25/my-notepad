//! `cargo xtask gen-samples`: 성능 측정용 샘플 파일 생성 (P0-06, P0-09)과 에디터 확인용 샘플 (P2-01).
//!
//! 결과는 `spikes/samples/`에 쓰며 `.gitignore` 대상이다. 같은 내용이 나오도록 고정 시드를 쓴다.

use std::fs;
use std::path::Path;

/// (파일 이름, 목표 크기 바이트)
const TEXT_SAMPLES: &[(&str, usize)] = &[
    ("text-1mb.txt", 1 << 20),
    ("text-5mb.txt", 5 << 20),
    ("text-10mb.txt", 10 << 20),
    ("text-50mb.txt", 50 << 20),
];

const MD_SAMPLES: &[(&str, usize)] = &[("md-100kb.md", 100 << 10), ("md-1mb.md", 1 << 20)];

const WORDS_KO: &[&str] = &[
    "인코딩",
    "파일",
    "저장",
    "검색",
    "한글",
    "에디터",
    "탭",
    "설정",
    "사용자",
    "문서",
    "변환",
    "줄바꿈",
    "미리보기",
    "성능",
    "측정",
    "결과",
    "오류",
    "경고",
    "정보",
    "요청",
];
const WORDS_EN: &[&str] = &[
    "server", "request", "encoding", "buffer", "layout", "render", "cache", "window", "thread",
    "parser", "token", "error", "value", "index", "status", "config",
];
const LEVELS: &[&str] = &["INFO ", "INFO ", "INFO ", "WARN ", "ERROR", "DEBUG"];

/// 고정 시드 선형 합동 생성기 (외부 의존성 없이 재현 가능한 내용)
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as usize
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.next() % items.len()]
    }
}

/// 로그 형식의 한·영 혼합 줄 (`ERROR`가 섞여 있어 하이라이트 측정에 쓴다)
fn text_line(rng: &mut Lcg, n: usize) -> String {
    let mut line = format!(
        "2026-10-20 {:02}:{:02}:{:02} {} [{:>6}] ",
        (n / 3600) % 24,
        (n / 60) % 60,
        n % 60,
        rng.pick(LEVELS),
        n
    );
    let words = 6 + rng.next() % 8;
    for i in 0..words {
        if i > 0 {
            line.push(' ');
        }
        if rng.next().is_multiple_of(2) {
            line.push_str(rng.pick(WORDS_KO));
        } else {
            line.push_str(rng.pick(WORDS_EN));
        }
    }
    line.push('\n');
    line
}

pub fn generate(root: &Path) -> Result<(), String> {
    let dir = root.join("spikes/samples");
    fs::create_dir_all(&dir).map_err(|e| format!("{} 생성 실패: {e}", dir.display()))?;

    for &(name, size) in TEXT_SAMPLES {
        let mut rng = Lcg(0x5EED_0001);
        let mut text = String::with_capacity(size + 256);
        let mut n = 0;
        while text.len() < size {
            text.push_str(&text_line(&mut rng, n));
            n += 1;
        }
        write(&dir.join(name), &text, n)?;
    }

    // Markdown 샘플: 실제 기획 문서를 목표 크기까지 반복 (표·코드·목록·Mermaid 포함)
    let plan = fs::read_to_string(root.join("docs/PLAN.md"))
        .map_err(|e| format!("docs/PLAN.md 읽기 실패: {e}"))?;
    for &(name, size) in MD_SAMPLES {
        let mut text = String::with_capacity(size + plan.len());
        let mut copy = 1;
        while text.len() < size {
            text.push_str(&format!("\n\n<!-- 반복 {copy} -->\n\n"));
            text.push_str(&plan);
            copy += 1;
        }
        let lines = text.lines().count();
        write(&dir.join(name), &text, lines)?;
    }

    // 에디터 확인용 (P2-01): 줄 번호 Gutter 폭(10,000줄 초과), 자동 줄바꿈 켬/끔
    let mut text = String::new();
    for n in 1..=12_000 {
        text.push_str(&format!("{n}번째 줄 — 번호 확인\n"));
    }
    write(&dir.join("lines-12k.txt"), &text, 12_000)?;

    let mut rng = Lcg(0x5EED_0002);
    let mut text = String::new();
    for n in 0..300 {
        if n % 5 == 2 {
            text.push_str(&format!("[{:>3}] 긴 줄: ", n + 1));
            for _ in 0..70 {
                text.push_str(rng.pick(WORDS_KO));
                text.push(' ');
                text.push_str(rng.pick(WORDS_EN));
                text.push(' ');
            }
            text.push_str("← 줄 끝\n");
        } else {
            text.push_str(&text_line(&mut rng, n));
        }
    }
    write(&dir.join("long-lines.txt"), &text, 300)?;

    println!("\n✔ {}", dir.display());
    Ok(())
}

fn write(path: &Path, text: &str, lines: usize) -> Result<(), String> {
    fs::write(path, text).map_err(|e| format!("{} 쓰기 실패: {e}", path.display()))?;
    println!(
        "  {:<16} {:>8.2} MB  {:>9} 줄",
        path.file_name().unwrap_or_default().to_string_lossy(),
        text.len() as f64 / 1_048_576.0,
        lines
    );
    Ok(())
}
