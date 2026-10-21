//! OS별 시스템 폰트 탐색 및 Fallback 체인 구성 (PLAN.md §2.4 · WORKPLAN.md P1-02).
//!
//! - `Monospace`    = `[mono, ko]` + egui 기본 폰트
//! - `Proportional` = `[ui, ko]` + egui 기본 폰트
//! - `bold`         = `[ui-bold, ko-bold]` + egui 기본 Proportional 체인
//! - `file-icons`   = 내장 Phosphor 단색 아이콘 폰트 (`assets/file-icons/phosphor/Phosphor.ttf`)
//!
//! 시스템 폰트 파일은 메모리에 복사하지 않고 memory-map 하며, `.ttc` 컬렉션은 이름/굵기 테이블로
//! 알맞은 face를 선택한다. 후보 경로 중 앞선 경로가 없으면 다음 후보를 탐색하고,
//! 모두 실패하면 egui 기본 폰트로 기동하되 상태 표시줄과 로그에 경고를 남긴다.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{FontData, FontDefinitions, FontFamily};

/// 굵은 글씨용 사용자 정의 패밀리 이름.
pub const BOLD_FAMILY: &str = "bold";

/// 파일·폴더·UI 컨트롤 아이콘용 패밀리 이름 (Phosphor).
pub const ICON_FAMILY: &str = "file-icons";

/// 폰트 슬롯 한 개의 로드 결과.
#[derive(Clone, Debug)]
pub struct LoadedFont {
    pub key: &'static str,
    pub path: PathBuf,
    pub face_index: u32,
    pub face_name: String,
    pub size_bytes: u64,
    pub elapsed: Duration,
}

/// 폰트 체인 구성 결과 보고서.
#[derive(Clone, Debug, Default)]
pub struct FontReport {
    pub loaded: Vec<LoadedFont>,
    pub failures: Vec<String>,
    pub total: Duration,
}

impl FontReport {
    /// 사람이 읽을 수 있는 요약 보고서 문자열을 반환한다.
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
                font.path.display(),
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

    /// 하나 이상의 시스템 폰트 슬롯이 로드에 실패했는지 여부.
    pub fn has_failures(&self) -> bool {
        !self.failures.is_empty()
    }

    /// 상태 표시줄에 표시할 짧은 경고 문구 (정상일 때는 `None`).
    pub fn status_warning(&self) -> Option<String> {
        if self.failures.is_empty() {
            None
        } else {
            Some(format!(
                "⚠ 일부 시스템 글꼴 로드 실패 ({}건) — 기본 글꼴로 대체됨",
                self.failures.len()
            ))
        }
    }
}

/// 체인에 넣을 폰트 슬롯 정의: 역할 키, 찾을 굵기 여부, 우선순위 후보 경로 목록.
#[derive(Clone, Debug)]
pub struct FontSlot {
    pub key: &'static str,
    pub bold: bool,
    pub candidates: Vec<PathBuf>,
}

/// OS 시스템 폰트 후보 목록을 생성한다.
pub fn os_font_slots() -> Vec<FontSlot> {
    #[cfg(target_os = "windows")]
    {
        let windir = std::env::var_os("WINDIR").map(PathBuf::from);
        windows_font_slots(windir.as_deref())
    }
    #[cfg(target_os = "macos")]
    {
        macos_font_slots()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Vec::new()
    }
}

/// Windows 폰트 디렉토리(`%WINDIR%\Fonts`, 미설정 시 `C:\Windows\Fonts`)를 기준으로 후보 목록을 만든다.
#[cfg(any(target_os = "windows", test))]
pub fn windows_font_slots(windir: Option<&Path>) -> Vec<FontSlot> {
    let fonts_dir = windir
        .unwrap_or_else(|| Path::new(r"C:\Windows"))
        .join("Fonts");
    vec![
        FontSlot {
            key: "mono",
            bold: false,
            candidates: vec![fonts_dir.join("consola.ttf"), fonts_dir.join("cour.ttf")],
        },
        FontSlot {
            key: "ui",
            bold: false,
            candidates: vec![fonts_dir.join("segoeui.ttf"), fonts_dir.join("malgun.ttf")],
        },
        FontSlot {
            key: "ui-bold",
            bold: true,
            candidates: vec![
                fonts_dir.join("segoeuib.ttf"),
                fonts_dir.join("malgunbd.ttf"),
            ],
        },
        FontSlot {
            key: "ko",
            bold: false,
            candidates: vec![fonts_dir.join("malgun.ttf"), fonts_dir.join("gulim.ttc")],
        },
        FontSlot {
            key: "ko-bold",
            bold: true,
            candidates: vec![fonts_dir.join("malgunbd.ttf"), fonts_dir.join("malgun.ttf")],
        },
    ]
}

/// macOS 시스템 폰트 후보 목록을 만든다.
#[cfg(any(target_os = "macos", test))]
pub fn macos_font_slots() -> Vec<FontSlot> {
    let sys_fonts = Path::new("/System/Library/Fonts");
    let supp_fonts = sys_fonts.join("Supplemental");
    vec![
        FontSlot {
            key: "mono",
            bold: false,
            candidates: vec![sys_fonts.join("Menlo.ttc"), sys_fonts.join("Monaco.ttf")],
        },
        FontSlot {
            key: "ui",
            bold: false,
            candidates: vec![
                sys_fonts.join("AppleSDGothicNeo.ttc"),
                supp_fonts.join("AppleGothic.ttf"),
            ],
        },
        FontSlot {
            key: "ui-bold",
            bold: true,
            candidates: vec![
                sys_fonts.join("AppleSDGothicNeo.ttc"),
                supp_fonts.join("AppleGothic.ttf"),
            ],
        },
        FontSlot {
            key: "ko",
            bold: false,
            candidates: vec![
                sys_fonts.join("AppleSDGothicNeo.ttc"),
                supp_fonts.join("AppleGothic.ttf"),
            ],
        },
        FontSlot {
            key: "ko-bold",
            bold: true,
            candidates: vec![
                sys_fonts.join("AppleSDGothicNeo.ttc"),
                supp_fonts.join("AppleGothic.ttf"),
            ],
        },
    ]
}

/// 후보 경로 목록에서 첫 번째로 존재하는 파일 경로를 반환한다.
/// 존재하는 파일이 하나도 없으면 `Err`에 시도한 경로 목록을 담아 돌려준다.
pub fn resolve_candidate_path(candidates: &[PathBuf]) -> Result<PathBuf, String> {
    for path in candidates {
        if path.is_file() {
            return Ok(path.clone());
        }
    }
    let tried: Vec<String> = candidates.iter().map(|p| p.display().to_string()).collect();
    Err(format!(
        "존재하는 폰트 파일 없음 (시도: {})",
        tried.join(", ")
    ))
}

/// OS 시스템 폰트 Fallback 체인과 내장 아이콘 폰트를 구성한 `FontDefinitions`와 `FontReport`를 반환한다.
pub fn explicit_chain() -> (FontDefinitions, FontReport) {
    build_chain(&os_font_slots())
}

/// 주어진 폰트 슬롯 목록으로 `FontDefinitions`와 `FontReport`를 구성한다 (테스트에서 임의 경로 주입 가능).
pub fn build_chain(slots: &[FontSlot]) -> (FontDefinitions, FontReport) {
    let started = Instant::now();
    let mut defs = FontDefinitions::default();
    let mut report = FontReport::default();

    for slot in slots {
        let t = Instant::now();
        match load_slot(slot) {
            Ok((data, path, face_index, face_name, size_bytes)) => {
                defs.font_data.insert(slot.key.to_owned(), Arc::new(data));
                report.loaded.push(LoadedFont {
                    key: slot.key,
                    path,
                    face_index,
                    face_name,
                    size_bytes,
                    elapsed: t.elapsed(),
                });
            }
            Err(err) => {
                report.failures.push(format!("{}: {err}", slot.key));
            }
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

    register_icon_font(&mut defs);

    report.total = started.elapsed();
    (defs, report)
}

/// 내장 Phosphor 아이콘 폰트를 `ICON_FAMILY` 패밀리로 등록한다.
pub fn register_icon_font(defs: &mut FontDefinitions) {
    defs.font_data.insert(
        "phosphor".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/file-icons/phosphor/Phosphor.ttf"
        ))),
    );
    defs.families.insert(
        FontFamily::Name(ICON_FAMILY.into()),
        vec!["phosphor".to_owned()],
    );
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

/// 슬롯의 후보 경로를 순서대로 시도해 첫 번째로 유효한 폰트 파일을 memory-map 하여 로드한다.
fn load_slot(slot: &FontSlot) -> Result<(FontData, PathBuf, u32, String, u64), String> {
    let first_existing = resolve_candidate_path(&slot.candidates)?;
    let mut errors = Vec::new();
    for candidate in &slot.candidates {
        if !candidate.is_file() {
            continue;
        }
        match load_file(candidate, slot.bold) {
            Ok((data, face_index, face_name, size_bytes)) => {
                return Ok((data, candidate.clone(), face_index, face_name, size_bytes));
            }
            Err(err) => {
                errors.push(format!("{} ({err})", candidate.display()));
            }
        }
    }
    Err(format!(
        "{} 로드 실패: {}",
        first_existing.display(),
        errors.join(" → ")
    ))
}

/// 파일을 memory-map 하고, 컬렉션(.ttc)이면 원하는 스타일의 face를 찾는다.
fn load_file(path: &Path, bold: bool) -> Result<(FontData, u32, String, u64), String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let size_bytes = file.metadata().map_err(|e| e.to_string())?.len();
    // SAFETY: 시스템 폰트 파일은 앱 실행 중 바뀌지 않는다고 가정한다 (egui_system_font_provider와 같은 방식).
    let mmap = unsafe { memmap2::Mmap::map(&file) }.map_err(|e| e.to_string())?;

    let (face_index, face_name) = pick_face(&mmap, bold)?;
    let data = FontData::from_blob(Arc::new(mmap), face_index);
    Ok((data, face_index, face_name, size_bytes))
}

/// 파일 안의 face 중 굵기 조건에 맞는 첫 face를 고른다. 단일 폰트 파일이면 0번.
pub fn pick_face(bytes: &[u8], bold: bool) -> Result<(u32, String), String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_slots_use_custom_windir() {
        let custom = Path::new(r"D:\CustomWin");
        let slots = windows_font_slots(Some(custom));
        assert_eq!(slots.len(), 5);
        assert_eq!(
            slots[0].candidates[0],
            PathBuf::from(r"D:\CustomWin\Fonts\consola.ttf")
        );
        assert_eq!(
            slots[3].candidates[0],
            PathBuf::from(r"D:\CustomWin\Fonts\malgun.ttf")
        );

        let mac_slots = macos_font_slots();
        assert_eq!(mac_slots.len(), 5);
        assert!(mac_slots.iter().all(|s| s.candidates.len() >= 2));
    }

    #[test]
    fn candidate_resolution_falls_back_to_next_existing_path() {
        let missing = PathBuf::from("/nonexistent/path/definitely_missing_font_12345.ttf");
        let existing =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/file-icons/phosphor/Phosphor.ttf");
        assert!(existing.is_file());

        let resolved = resolve_candidate_path(&[missing.clone(), existing.clone()])
            .expect("첫 후보가 없어도 두 번째 후보를 찾아야 함");
        assert_eq!(resolved, existing);

        let all_missing = resolve_candidate_path(&[missing]);
        assert!(all_missing.is_err());
    }

    #[test]
    fn build_chain_falls_back_to_second_candidate_and_handles_total_failure() {
        let missing = PathBuf::from("/nonexistent/path/missing_font.ttf");
        let phosphor =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/file-icons/phosphor/Phosphor.ttf");

        let slots = vec![
            FontSlot {
                key: "mono",
                bold: false,
                candidates: vec![missing.clone(), phosphor],
            },
            FontSlot {
                key: "ko",
                bold: false,
                candidates: vec![missing],
            },
        ];

        let (defs, report) = build_chain(&slots);
        // mono는 첫 번째 후보 실패 후 두 번째 후보(Phosphor.ttf)로 로드 성공해야 함
        assert_eq!(report.loaded.len(), 1);
        assert_eq!(report.loaded[0].key, "mono");
        // ko는 모든 후보 실패로 failures에 기록되지만 기본 폰트 체인은 유지되어야 함
        assert_eq!(report.failures.len(), 1);
        assert!(report.has_failures());
        assert!(report.status_warning().is_some());
        assert!(defs.families.contains_key(&FontFamily::Monospace));
        assert!(defs.families.contains_key(&FontFamily::Proportional));
        assert!(
            defs.families
                .contains_key(&FontFamily::Name(BOLD_FAMILY.into()))
        );
        assert!(
            defs.families
                .contains_key(&FontFamily::Name(ICON_FAMILY.into()))
        );
    }
}
