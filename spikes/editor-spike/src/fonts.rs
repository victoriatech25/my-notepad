//! P0-02 폰트 Fallback 체인 (PLAN.md §2.4).
//!
//! - Monospace    = [영문 고정폭, 한글] + egui 기본 폰트
//! - Proportional = [영문 UI, 한글] + egui 기본 폰트
//! - `bold`       = [영문 UI 굵게, 한글 굵게] + egui 기본 폰트 (egui는 가짜 굵게를 그리지 않음)
//!
//! 시스템 폰트 파일은 메모리에 복사하지 않고 memory-map 한다 (한글 폰트는 파일당 10MB 이상).

use std::fs::File;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{FontData, FontDefinitions, FontFamily};

/// 굵은 글씨용 사용자 정의 패밀리 이름.
pub const BOLD_FAMILY: &str = "bold";

/// 폰트 한 개의 로드 결과.
pub struct LoadedFont {
    pub key: &'static str,
    pub path: &'static str,
    pub face_index: u32,
    pub face_name: String,
    pub size_bytes: u64,
    pub elapsed: Duration,
}

/// 폰트 체인 구성 결과 보고서.
#[derive(Default)]
pub struct FontReport {
    pub loaded: Vec<LoadedFont>,
    pub failures: Vec<String>,
    pub total: Duration,
}

impl FontReport {
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for font in &self.loaded {
            out.push_str(&format!(
                "{:<10} {:>6.2} MB {:>7.2} ms  face {} \"{}\"  {}\n",
                font.key,
                font.size_bytes as f64 / 1_048_576.0,
                font.elapsed.as_secs_f64() * 1000.0,
                font.face_index,
                font.face_name,
                font.path,
            ));
        }
        for failure in &self.failures {
            out.push_str(&format!("FAILED     {failure}\n"));
        }
        out.push_str(&format!(
            "total      {:.2} ms\n",
            self.total.as_secs_f64() * 1000.0
        ));
        out
    }
}

/// 체인에 넣을 폰트 후보: (키, 경로, 찾을 스타일)
struct Candidate {
    key: &'static str,
    path: &'static str,
    bold: bool,
}

#[cfg(target_os = "windows")]
const CANDIDATES: &[Candidate] = &[
    Candidate {
        key: "mono",
        path: r"C:\Windows\Fonts\consola.ttf",
        bold: false,
    },
    Candidate {
        key: "ui",
        path: r"C:\Windows\Fonts\segoeui.ttf",
        bold: false,
    },
    Candidate {
        key: "ui-bold",
        path: r"C:\Windows\Fonts\segoeuib.ttf",
        bold: true,
    },
    Candidate {
        key: "ko",
        path: r"C:\Windows\Fonts\malgun.ttf",
        bold: false,
    },
    Candidate {
        key: "ko-bold",
        path: r"C:\Windows\Fonts\malgunbd.ttf",
        bold: true,
    },
];

#[cfg(target_os = "macos")]
const CANDIDATES: &[Candidate] = &[
    Candidate {
        key: "mono",
        path: "/System/Library/Fonts/Menlo.ttc",
        bold: false,
    },
    Candidate {
        key: "ui",
        path: "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        bold: false,
    },
    Candidate {
        key: "ui-bold",
        path: "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        bold: true,
    },
    Candidate {
        key: "ko",
        path: "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        bold: false,
    },
    Candidate {
        key: "ko-bold",
        path: "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        bold: true,
    },
];

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const CANDIDATES: &[Candidate] = &[];

/// egui 기본 폰트 앞에 시스템 폰트를 끼워 넣은 정의와 보고서를 만든다.
/// 로드에 실패한 폰트는 건너뛰고 보고서에 남긴다 (기본 폰트로는 항상 기동).
pub fn explicit_chain() -> (FontDefinitions, FontReport) {
    let started = Instant::now();
    let mut defs = FontDefinitions::default();
    let mut report = FontReport::default();

    for candidate in CANDIDATES {
        let t = Instant::now();
        match load(candidate) {
            Ok((data, face_index, face_name, size_bytes)) => {
                defs.font_data
                    .insert(candidate.key.to_owned(), Arc::new(data));
                report.loaded.push(LoadedFont {
                    key: candidate.key,
                    path: candidate.path,
                    face_index,
                    face_name,
                    size_bytes,
                    elapsed: t.elapsed(),
                });
            }
            Err(err) => report
                .failures
                .push(format!("{} ({}): {err}", candidate.key, candidate.path)),
        }
    }

    let has = |key: &str| defs.font_data.contains_key(key);
    let chain = |keys: &[&str]| -> Vec<String> {
        keys.iter()
            .filter(|k| has(k))
            .map(|k| (*k).to_owned())
            .collect()
    };

    let mono = chain(&["mono", "ko"]);
    let proportional = chain(&["ui", "ko"]);
    let bold = chain(&["ui-bold", "ko-bold"]);

    prepend(&mut defs, FontFamily::Monospace, mono);
    prepend(&mut defs, FontFamily::Proportional, proportional);

    // 굵게 패밀리는 egui 기본 Proportional 체인을 뒤에 붙여 기호·이모지를 유지한다.
    let mut bold_family = bold;
    bold_family.extend(defs.families[&FontFamily::Proportional].iter().cloned());
    defs.families
        .insert(FontFamily::Name(BOLD_FAMILY.into()), dedup(bold_family));

    report.total = started.elapsed();
    (defs, report)
}

/// 비교용: egui 기본 폰트만 쓰고 한글은 eframe의 시스템 폰트 자동 대체에 맡긴다.
/// `bold` 패밀리는 존재해야 하므로 Proportional 체인을 그대로 복사한다.
pub fn egui_default() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    let proportional = defs.families[&FontFamily::Proportional].clone();
    defs.families
        .insert(FontFamily::Name(BOLD_FAMILY.into()), proportional);
    defs
}

fn prepend(defs: &mut FontDefinitions, family: FontFamily, mut keys: Vec<String>) {
    let existing = defs.families.entry(family).or_default();
    keys.append(existing);
    *existing = dedup(keys);
}

fn dedup(keys: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(keys.len());
    for key in keys {
        if !out.contains(&key) {
            out.push(key);
        }
    }
    out
}

/// 파일을 memory-map 하고, 컬렉션(.ttc)이면 원하는 스타일의 face를 찾는다.
fn load(candidate: &Candidate) -> Result<(FontData, u32, String, u64), String> {
    let path = Path::new(candidate.path);
    let file = File::open(path).map_err(|e| e.to_string())?;
    let size_bytes = file.metadata().map_err(|e| e.to_string())?.len();
    // SAFETY: 시스템 폰트 파일은 앱 실행 중 바뀌지 않는다고 가정한다 (egui_system_font_provider와 같은 방식).
    let mmap = unsafe { memmap2::Mmap::map(&file) }.map_err(|e| e.to_string())?;

    let (face_index, face_name) = pick_face(&mmap, candidate.bold)?;
    let data = FontData::from_blob(Arc::new(mmap), face_index);
    Ok((data, face_index, face_name, size_bytes))
}

/// 파일 안의 face 중 굵기 조건에 맞는 첫 face를 고른다. 단일 폰트 파일이면 0번.
fn pick_face(bytes: &[u8], bold: bool) -> Result<(u32, String), String> {
    let count = ttf_parser::fonts_in_collection(bytes).unwrap_or(1);
    let mut first: Option<(u32, String)> = None;
    for index in 0..count {
        let Ok(face) = ttf_parser::Face::parse(bytes, index) else {
            continue;
        };
        let name = face_name(&face);
        if first.is_none() {
            first = Some((index, name.clone()));
        }
        let is_bold = face.is_bold() || face.weight().to_number() >= 700;
        let is_regular = !face.is_italic() && face.weight().to_number() == 400;
        if (bold && is_bold && !face.is_italic()) || (!bold && is_regular) {
            return Ok((index, name));
        }
    }
    first.ok_or_else(|| "폰트 face를 읽을 수 없음".to_owned())
}

fn face_name(face: &ttf_parser::Face<'_>) -> String {
    face.names()
        .into_iter()
        .filter(|n| n.name_id == ttf_parser::name_id::FULL_NAME && n.is_unicode())
        .find_map(|n| n.to_string())
        .unwrap_or_else(|| "?".to_owned())
}
