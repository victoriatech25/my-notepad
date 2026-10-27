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
//!
//! 편집기 글꼴 (P5-UX, `보기 → 글꼴…`): 사용자가 고른 글꼴을 `Monospace` 체인 맨 앞에 [`EDITOR_FONT`]로 넣는다.
//! 그 글꼴에 없는 글자(한글 등)는 뒤의 기본 체인(`mono`, `ko`)으로 그린다. 글꼴 목록은 [`scan_fonts`]가 OS 글꼴
//! 폴더의 이름 테이블만 읽어 만든다 (대화상자를 열 때 작업 스레드에서).

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

/// 고정폭 패밀리 글꼴들의 탭 문자 폭을 공백 `spaces`칸으로 바꾼다 (P2-02 탭 너비).
///
/// egui는 탭을 "공백 N칸" 고정 폭으로 그린다(`FontTweak::tab_size`). 바꾼 정의는 `Context::set_fonts`로 다시 적용해야 한다.
pub fn set_tab_size(defs: &mut FontDefinitions, spaces: f32) {
    let keys = defs
        .families
        .get(&FontFamily::Monospace)
        .cloned()
        .unwrap_or_default();
    for key in keys {
        if let Some(data) = defs.font_data.get_mut(&key)
            && data.tweak.tab_size != spaces
        {
            Arc::make_mut(data).tweak.tab_size = spaces;
        }
    }
}

/// 편집기 기본 고정폭 글꼴 이름 (글꼴 대화상자의 "기본" 항목).
pub const DEFAULT_MONO_NAME: &str = if cfg!(target_os = "macos") {
    "Menlo"
} else {
    "Consolas"
};

/// 사용자가 고른 편집기 글꼴의 글꼴 데이터 키 (`Monospace` 체인 맨 앞).
pub const EDITOR_FONT: &str = "editor";
/// 글꼴 대화상자 미리보기용 패밀리 (고르는 중인 글꼴 + 기본 고정폭 체인).
pub const FONT_PREVIEW_FAMILY: &str = "font-preview";
const PREVIEW_FONT: &str = "font-preview";

/// 설치된 글꼴 하나 (글꼴 대화상자 목록).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FontEntry {
    /// 보이는 이름 (한국어 이름이 있으면 한국어 — "맑은 고딕")
    pub name: String,
    pub path: PathBuf,
    /// `.ttc` 컬렉션 안의 face 번호 (보통 굵기·기울임 없는 face)
    pub face: u32,
    /// 고정폭 글꼴 (`post` 테이블 `isFixedPitch`)
    pub monospace: bool,
}

/// OS 글꼴 폴더 (시스템 + 사용자 설치).
pub fn font_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(target_os = "windows")]
    {
        let windir =
            std::env::var_os("WINDIR").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
        dirs.push(windir.join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join(r"Microsoft\Windows\Fonts"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        dirs.push(PathBuf::from("/System/Library/Fonts"));
        dirs.push(PathBuf::from("/Library/Fonts"));
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join("Library/Fonts"));
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        dirs.push(PathBuf::from("/usr/share/fonts"));
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(home).join(".local/share/fonts"));
        }
    }
    dirs
}

/// 글꼴 폴더들(하위 폴더 포함)의 글꼴을 이름순으로 모은다. 같은 이름은 하나만 (보통 굵기 face를 우선).
/// 기호 글꼴처럼 영문 `a`가 없는 글꼴은 편집기에 쓸 수 없어 뺀다.
pub fn scan_fonts(dirs: &[PathBuf]) -> Vec<FontEntry> {
    let mut found: Vec<(FontEntry, bool)> = Vec::new();
    let mut stack: Vec<PathBuf> = dirs.to_vec();
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let is_font = path.extension().is_some_and(|e| {
                ["ttf", "otf", "ttc", "otc"]
                    .iter()
                    .any(|x| e.eq_ignore_ascii_case(x))
            });
            if is_font {
                read_font_faces(&path, &mut found);
            }
        }
    }
    // 같은 이름: 보통 굵기 face가 있으면 그것, 없으면 처음 본 것
    found.sort_by(|(a, a_regular), (b, b_regular)| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(b_regular.cmp(a_regular))
            .then(a.path.cmp(&b.path))
    });
    found.dedup_by(|(a, _), (b, _)| a.name == b.name);
    found.into_iter().map(|(entry, _)| entry).collect()
}

/// 글꼴 파일 하나의 face들을 (항목, 보통 굵기인지)로 모은다.
fn read_font_faces(path: &Path, out: &mut Vec<(FontEntry, bool)>) {
    let Ok(file) = File::open(path) else {
        return;
    };
    // SAFETY: `load_file`과 같다 — 이름 테이블만 읽고 바로 놓는다.
    let Ok(mmap) = (unsafe { memmap2::Mmap::map(&file) }) else {
        return;
    };
    let count = ttf_parser::fonts_in_collection(&mmap).unwrap_or(1);
    for face_index in 0..count {
        let Ok(face) = ttf_parser::Face::parse(&mmap, face_index) else {
            continue;
        };
        if face.glyph_index('a').is_none() {
            continue;
        }
        let Some(name) = family_name(&face) else {
            continue;
        };
        let regular = !face.is_italic() && face.weight().to_number() == 400;
        out.push((
            FontEntry {
                name,
                path: path.to_path_buf(),
                face: face_index,
                monospace: face.is_monospaced(),
            },
            regular,
        ));
    }
}

/// 글꼴 패밀리 이름: 한국어(0x0412) → 영어(0x0409) → 아무 유니코드 이름. 굵기·기울임 이름이 붙지 않은
/// "대표 패밀리"(ID 16)가 있으면 그것을 쓴다.
fn family_name(face: &ttf_parser::Face<'_>) -> Option<String> {
    use ttf_parser::name_id::{FAMILY, TYPOGRAPHIC_FAMILY};
    let names: Vec<_> = face
        .names()
        .into_iter()
        .filter(|n| n.is_unicode())
        .collect();
    for id in [TYPOGRAPHIC_FAMILY, FAMILY] {
        let of_id = || names.iter().filter(move |n| n.name_id == id);
        let pick = of_id()
            .find(|n| n.language_id == 0x0412)
            .or_else(|| of_id().find(|n| n.language_id == 0x0409))
            .or_else(|| of_id().next());
        if let Some(name) = pick.and_then(|n| n.to_string())
            && !name.trim().is_empty()
        {
            return Some(name.trim().to_owned());
        }
    }
    // 오래된 Apple 글꼴(Menlo 등)은 이름을 Mac 플랫폼(Mac Roman) 기록에만 두기도 한다 — 영문 이름은 ASCII와 같다
    for id in [TYPOGRAPHIC_FAMILY, FAMILY] {
        let name = face
            .names()
            .into_iter()
            .find(|n| {
                n.name_id == id
                    && n.platform_id == ttf_parser::PlatformId::Macintosh
                    && n.encoding_id == 0
            })
            .and_then(|n| mac_roman_ascii(n.name));
        if name.is_some() {
            return name;
        }
    }
    None
}

/// Mac Roman 이름 기록을 글자로 바꾼다 (ASCII만 — 그 밖의 글자가 있으면 `None`).
fn mac_roman_ascii(bytes: &[u8]) -> Option<String> {
    let text: String = bytes
        .iter()
        .map(|&b| b.is_ascii().then_some(char::from(b)))
        .collect::<Option<_>>()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// 편집기 글꼴을 `Monospace` 체인 맨 앞에 넣는다 (`None`이면 빼고 기본 체인으로). 바꾼 정의는
/// `Context::set_fonts`로 다시 적용해야 한다. 읽지 못하면 기본 체인으로 두고 이유를 돌려준다.
pub fn set_editor_font(
    defs: &mut FontDefinitions,
    font: Option<(&Path, u32)>,
    tab_size: f32,
) -> Result<(), String> {
    remove_from_chain(defs, &FontFamily::Monospace, EDITOR_FONT);
    let Some((path, face)) = font else {
        return Ok(());
    };
    let mut data = load_face(path, face)?;
    data.tweak.tab_size = tab_size;
    defs.font_data
        .insert(EDITOR_FONT.to_owned(), Arc::new(data));
    if let Some(chain) = defs.families.get_mut(&FontFamily::Monospace) {
        chain.insert(0, EDITOR_FONT.to_owned());
    }
    Ok(())
}

/// 글꼴 대화상자 미리보기 패밀리를 `font`(`None`이면 기본 체인)로 맞춘다.
pub fn set_preview_font(
    defs: &mut FontDefinitions,
    font: Option<(&Path, u32)>,
) -> Result<(), String> {
    let family = FontFamily::Name(FONT_PREVIEW_FAMILY.into());
    // 기본 체인 = 편집기 글꼴을 뺀 Monospace 체인
    let mut chain: Vec<String> = defs
        .families
        .get(&FontFamily::Monospace)
        .map(|c| c.iter().filter(|k| *k != EDITOR_FONT).cloned().collect())
        .unwrap_or_default();
    defs.font_data.remove(PREVIEW_FONT);
    let result = match font {
        Some((path, face)) => load_face(path, face).map(|data| {
            defs.font_data
                .insert(PREVIEW_FONT.to_owned(), Arc::new(data));
            chain.insert(0, PREVIEW_FONT.to_owned());
        }),
        None => Ok(()),
    };
    defs.families.insert(family, chain);
    result
}

fn remove_from_chain(defs: &mut FontDefinitions, family: &FontFamily, key: &str) {
    defs.font_data.remove(key);
    if let Some(chain) = defs.families.get_mut(family) {
        chain.retain(|k| k != key);
    }
}

/// 글꼴 파일의 `face`번 face를 memory-map 해서 읽는다.
fn load_face(path: &Path, face: u32) -> Result<FontData, String> {
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // SAFETY: `load_file`과 같다.
    let mmap = unsafe { memmap2::Mmap::map(&file) }.map_err(|e| e.to_string())?;
    ttf_parser::Face::parse(&mmap, face)
        .map_err(|e| format!("{}: 글꼴을 읽을 수 없음 ({e})", path.display()))?;
    Ok(FontData::from_blob(Arc::new(mmap), face))
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

/// 파일 안의 face 중 굵기 조건에 맞는 face를 고른다. 단일 폰트 파일이면 0번.
///
/// - 보통: 굵기 400인 첫 face
/// - 굵게: 굵은 face(`is_bold` 또는 굵기 700 이상) 중 굵기가 700에 가장 가까운 face
///   (`.ttc`에 Bold·ExtraBold·Heavy가 함께 있을 때 파일 순서와 관계없이 Bold를 고른다)
///
/// 조건에 맞는 face가 없으면 첫 face를 돌려준다.
pub fn pick_face(bytes: &[u8], bold: bool) -> Result<(u32, String), String> {
    let count = ttf_parser::fonts_in_collection(bytes).unwrap_or(1);
    let mut first: Option<(u32, String)> = None;
    let mut best_bold: Option<(u16, u32, String)> = None;
    for index in 0..count {
        let Ok(face) = ttf_parser::Face::parse(bytes, index) else {
            continue;
        };
        let name = face_name(&face);
        if first.is_none() {
            first = Some((index, name.clone()));
        }
        if face.is_italic() {
            continue;
        }
        let weight = face.weight().to_number();
        if bold {
            if face.is_bold() || weight >= 700 {
                let distance = weight.abs_diff(700);
                if best_bold.as_ref().is_none_or(|(d, _, _)| distance < *d) {
                    best_bold = Some((distance, index, name));
                }
            }
        } else if weight == 400 {
            return Ok((index, name));
        }
    }
    best_bold
        .map(|(_, index, name)| (index, name))
        .or(first)
        .ok_or_else(|| "폰트 face를 읽을 수 없음".to_owned())
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
    fn tab_size_changes_only_monospace_fonts() {
        let mut defs = FontDefinitions::default();
        let mono: Vec<String> = defs.families[&FontFamily::Monospace].clone();
        // 고정폭 글꼴과 데이터를 공유하는 비례폭 전용 글꼴 (같은 `Arc`)
        let shared = Arc::clone(&defs.font_data[&mono[0]]);
        defs.font_data.insert("ui-only".to_owned(), shared);
        defs.families
            .get_mut(&FontFamily::Proportional)
            .unwrap()
            .insert(0, "ui-only".to_owned());

        set_tab_size(&mut defs, 8.0);
        for key in &mono {
            assert_eq!(defs.font_data[key].tweak.tab_size, 8.0, "{key}");
        }
        assert_eq!(
            defs.font_data["ui-only"].tweak.tab_size, 4.0,
            "데이터를 공유해도 비례폭 글꼴은 그대로"
        );
    }

    fn phosphor() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/file-icons/phosphor/Phosphor.ttf")
    }

    #[test]
    fn editor_font_goes_first_in_monospace_chain_and_can_be_removed() {
        let mut defs = FontDefinitions::default();
        let base = defs.families[&FontFamily::Monospace].clone();

        set_editor_font(&mut defs, Some((&phosphor(), 0)), 8.0).unwrap();
        let chain = &defs.families[&FontFamily::Monospace];
        assert_eq!(chain[0], EDITOR_FONT);
        assert_eq!(
            &chain[1..],
            &base[..],
            "기본 체인은 뒤에 그대로 (한글 등 대체)"
        );
        assert_eq!(defs.font_data[EDITOR_FONT].tweak.tab_size, 8.0);

        // 다시 골라도 한 번만
        set_editor_font(&mut defs, Some((&phosphor(), 0)), 4.0).unwrap();
        let count = defs.families[&FontFamily::Monospace]
            .iter()
            .filter(|k| *k == EDITOR_FONT)
            .count();
        assert_eq!(count, 1);

        // 미리보기 패밀리는 편집기 글꼴을 뺀 기본 체인 + 고르는 글꼴
        set_preview_font(&mut defs, None).unwrap();
        let preview = FontFamily::Name(FONT_PREVIEW_FAMILY.into());
        assert_eq!(defs.families[&preview], base);
        set_preview_font(&mut defs, Some((&phosphor(), 0))).unwrap();
        assert_eq!(defs.families[&preview][0], PREVIEW_FONT);

        set_editor_font(&mut defs, None, 4.0).unwrap();
        assert_eq!(defs.families[&FontFamily::Monospace], base);
        assert!(!defs.font_data.contains_key(EDITOR_FONT));

        // 없는 파일: 오류, 체인은 기본 그대로
        let missing = Path::new("/nonexistent/missing_font.ttf");
        assert!(set_editor_font(&mut defs, Some((missing, 0)), 4.0).is_err());
        assert_eq!(defs.families[&FontFamily::Monospace], base);
    }

    #[test]
    fn mac_roman_names_are_read_as_ascii() {
        assert_eq!(mac_roman_ascii(b" Menlo "), Some("Menlo".to_owned()));
        assert_eq!(mac_roman_ascii(b"Caf\x8E"), None, "ASCII 밖 글자는 버린다");
        assert_eq!(mac_roman_ascii(b"  "), None);
    }

    #[test]
    fn scan_reads_family_names_and_skips_missing_folders() {
        // 폴더 안 글꼴 아닌 파일(LICENSE 등)과 없는 폴더는 건너뛴다
        let dirs = [
            phosphor().parent().unwrap().to_path_buf(),
            PathBuf::from("/nonexistent/fonts"),
        ];
        let fonts = scan_fonts(&dirs);
        assert_eq!(fonts.len(), 1, "{fonts:?}");
        assert_eq!(fonts[0].path, phosphor());
        assert!(fonts[0].name.contains("Phosphor"), "{}", fonts[0].name);
    }

    /// 실제 OS 글꼴 폴더에서 기본 고정폭 글꼴을 찾는다 (CI Windows·macOS).
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    #[test]
    fn scan_finds_default_monospace_font() {
        let fonts = scan_fonts(&font_dirs());
        let default = fonts
            .iter()
            .find(|f| f.name == DEFAULT_MONO_NAME)
            .unwrap_or_else(|| {
                // 다시 실패하면 원인을 볼 수 있게 비슷한 이름을 같이 보인다
                let near: Vec<_> = fonts
                    .iter()
                    .filter(|f| {
                        f.name
                            .to_lowercase()
                            .contains(&DEFAULT_MONO_NAME[..3].to_lowercase())
                    })
                    .map(|f| format!("{} ({})", f.name, f.path.display()))
                    .collect();
                panic!(
                    "{DEFAULT_MONO_NAME} 없음 ({}개) — 비슷한 이름: {near:?}",
                    fonts.len()
                )
            });
        assert!(default.monospace);
        // 이름순·중복 없음
        for pair in fonts.windows(2) {
            assert!(pair[0].name.to_lowercase() <= pair[1].name.to_lowercase());
            assert_ne!(pair[0].name, pair[1].name);
        }
        // 고른 face로 실제로 읽힌다
        let mut defs = FontDefinitions::default();
        set_editor_font(&mut defs, Some((&default.path, default.face)), 4.0).unwrap();
    }

    #[test]
    fn windows_slots_use_custom_windir() {
        let custom = Path::new(r"D:\CustomWin");
        let slots = windows_font_slots(Some(custom));
        assert_eq!(slots.len(), 5);
        // 기대 경로도 `join`으로 만들어 macOS(구분자 `/`)에서도 같은 결과를 비교한다.
        let fonts_dir = custom.join("Fonts");
        assert_eq!(slots[0].candidates[0], fonts_dir.join("consola.ttf"));
        assert_eq!(slots[3].candidates[0], fonts_dir.join("malgun.ttf"));

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
