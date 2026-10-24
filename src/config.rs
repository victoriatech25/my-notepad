//! 사용자 설정 및 최근 열어본 파일 영속화 (PLAN.md §3.5·§3.7, P2-08).
//!
//! - 설정은 사람이 읽을 수 있는 TOML 파일 하나(`settings.toml`)에 둔다 (PLAN의 eframe `persistence` 대신 — egui 내부
//!   메모리와 섞이지 않고 테스트·손상 처리가 쉽다).
//!   - Windows: `%APPDATA%\My-Notepad\settings.toml`
//!   - macOS: `~/Library/Application Support/My-Notepad/settings.toml`
//!   - 그 밖: `~/.config/my-notepad/settings.toml`
//! - 없는 항목은 기본값으로 채우고, 범위를 벗어난 값은 고친다 ([`Settings::sanitize`]).
//! - 파일이 손상되어 읽을 수 없으면 기본값으로 시작하고, 손상된 파일은 `settings.bad.toml`로 옮겨 둔다.
//! - 저장은 원자적으로 한다 (같은 폴더 임시 파일 → 이름 바꾸기, [`crate::core::file_io::write_atomic`]).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::commands::ThemeMode;
use crate::core::file_io;

/// 최근 파일 목록 최대 개수.
pub const MAX_RECENT_FILES: usize = 10;

/// 창 위치·크기 (논리 픽셀). 최대화했으면 `maximized`이고 크기는 최대화 전 크기.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub maximized: bool,
}

/// 사용자 설정 전체.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub window: Option<WindowGeometry>,
    pub sidebar_open: bool,
    pub sidebar_width: f32,
    /// 사이드바에 마지막으로 연 폴더
    pub sidebar_folder: Option<PathBuf>,
    /// 에디터 글꼴 확대율 (1.0 = 100%)
    pub zoom: f32,
    pub word_wrap: bool,
    pub tab_width: u8,
    pub tab_spaces: bool,
    pub highlight_whole_word: bool,
    pub highlight_match_case: bool,
    pub theme: ThemeMode,
    /// 최근 연 파일 (최근 것이 앞)
    pub recent_files: Vec<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            window: None,
            sidebar_open: true,
            sidebar_width: 220.0,
            sidebar_folder: None,
            zoom: 1.0,
            word_wrap: true,
            tab_width: 4,
            tab_spaces: false,
            highlight_whole_word: true,
            highlight_match_case: true,
            theme: ThemeMode::System,
            recent_files: Vec::new(),
        }
    }
}

/// [`Settings::load`] 결과.
#[derive(Debug)]
pub struct Loaded {
    pub settings: Settings,
    /// 읽지 못해 기본값을 썼다면 사용자에게 알릴 문구
    pub problem: Option<String>,
}

impl Settings {
    /// OS별 기본 설정 파일 경로.
    pub fn default_path() -> Option<PathBuf> {
        #[cfg(target_os = "windows")]
        {
            let base = std::env::var_os("APPDATA").map(PathBuf::from).or_else(|| {
                std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData/Roaming"))
            })?;
            Some(base.join("My-Notepad").join("settings.toml"))
        }
        #[cfg(target_os = "macos")]
        {
            let home = std::env::var_os("HOME").map(PathBuf::from)?;
            Some(
                home.join("Library")
                    .join("Application Support")
                    .join("My-Notepad")
                    .join("settings.toml"),
            )
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            let home = std::env::var_os("HOME").map(PathBuf::from)?;
            Some(
                home.join(".config")
                    .join("my-notepad")
                    .join("settings.toml"),
            )
        }
    }

    /// 설정 파일을 읽는다. 없으면 기본값, 손상되었으면 기본값 + 손상 파일을 `settings.bad.toml`로 옮긴다.
    pub fn load(path: &Path) -> Loaded {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Loaded {
                    settings: Self::default(),
                    problem: None,
                };
            }
            Err(e) => {
                log::warn!("설정 파일을 읽지 못함 ({}): {e}", path.display());
                return Loaded {
                    settings: Self::default(),
                    problem: Some(format!(
                        "설정 파일을 읽지 못해 기본 설정으로 시작합니다: {e}"
                    )),
                };
            }
        };
        match toml::from_str::<Self>(&text) {
            Ok(mut settings) => {
                settings.sanitize();
                Loaded {
                    settings,
                    problem: None,
                }
            }
            Err(e) => {
                log::warn!("설정 파일 손상 ({}): {e}", path.display());
                let backup = path.with_file_name("settings.bad.toml");
                let kept = std::fs::rename(path, &backup).is_ok();
                Loaded {
                    settings: Self::default(),
                    problem: Some(if kept {
                        format!(
                            "설정 파일이 손상되어 기본 설정으로 시작합니다. 손상된 파일은 {}에 보관했습니다.",
                            backup.display()
                        )
                    } else {
                        "설정 파일이 손상되어 기본 설정으로 시작합니다.".to_owned()
                    }),
                }
            }
        }
    }

    /// 설정을 원자적으로 저장한다 (폴더가 없으면 만든다).
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        file_io::write_atomic(path, text.as_bytes())
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    /// 범위를 벗어난 값을 고친다 (손으로 고친 파일·이전 버전 대비).
    pub fn sanitize(&mut self) {
        self.zoom = if self.zoom.is_finite() {
            self.zoom.clamp(0.5, 3.0)
        } else {
            1.0
        };
        if ![2, 4, 8].contains(&self.tab_width) {
            self.tab_width = 4;
        }
        self.sidebar_width = if self.sidebar_width.is_finite() {
            self.sidebar_width.clamp(140.0, 480.0)
        } else {
            220.0
        };
        if let Some(w) = self.window
            && !(w.x.is_finite()
                && w.y.is_finite()
                && w.width.is_finite()
                && w.height.is_finite()
                && w.width >= 200.0
                && w.height >= 150.0)
        {
            self.window = None;
        }
        self.recent_files.truncate(MAX_RECENT_FILES);
    }

    /// 최근 파일 맨 앞에 넣는다 (같은 파일은 한 번만, 최대 [`MAX_RECENT_FILES`]개).
    pub fn add_recent(&mut self, path: &Path) {
        let key = same_file_key(path);
        self.recent_files.retain(|p| same_file_key(p) != key);
        self.recent_files.insert(0, path.to_path_buf());
        self.recent_files.truncate(MAX_RECENT_FILES);
    }
}

/// 같은 파일 판정 키 (실제 경로, 없으면 받은 경로).
fn same_file_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_round_trip_through_toml_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/settings.toml");
        let settings = Settings {
            window: Some(WindowGeometry {
                x: 40.0,
                y: 60.0,
                width: 1000.0,
                height: 700.0,
                maximized: true,
            }),
            sidebar_open: false,
            sidebar_width: 300.0,
            sidebar_folder: Some(PathBuf::from("C:/devel/프로젝트")),
            zoom: 1.3,
            word_wrap: false,
            tab_width: 8,
            tab_spaces: true,
            highlight_whole_word: false,
            highlight_match_case: false,
            theme: ThemeMode::Dark,
            recent_files: vec![PathBuf::from("a.txt"), PathBuf::from("한글.md")],
        };
        settings.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("theme = \"dark\""), "{text}");
        let loaded = Settings::load(&path);
        assert!(loaded.problem.is_none());
        assert_eq!(loaded.settings, settings);
    }

    #[test]
    fn missing_file_and_missing_fields_use_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = Settings::load(&dir.path().join("none.toml"));
        assert_eq!(loaded.settings, Settings::default());
        assert!(loaded.problem.is_none());

        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "word_wrap = false\n").unwrap();
        let loaded = Settings::load(&path);
        assert!(!loaded.settings.word_wrap);
        assert_eq!(loaded.settings.tab_width, 4);
        assert_eq!(
            loaded.settings.theme,
            ThemeMode::System,
            "기본은 OS 설정 따르기"
        );
    }

    #[test]
    fn corrupted_file_starts_with_defaults_and_is_kept_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "zoom = [이건 TOML이 아님\n").unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.settings, Settings::default());
        assert!(loaded.problem.unwrap().contains("손상"));
        assert!(!path.exists());
        assert!(dir.path().join("settings.bad.toml").exists());

        // 타입이 틀린 값도 손상으로 본다
        std::fs::write(&path, "zoom = \"크게\"\n").unwrap();
        assert!(Settings::load(&path).problem.is_some());
    }

    #[test]
    fn out_of_range_values_are_fixed() {
        let mut s = Settings {
            zoom: 9.0,
            tab_width: 3,
            sidebar_width: 5.0,
            window: Some(WindowGeometry {
                x: 0.0,
                y: 0.0,
                width: 10.0,
                height: 10.0,
                maximized: false,
            }),
            recent_files: (0..20).map(|i| PathBuf::from(format!("{i}.txt"))).collect(),
            ..Settings::default()
        };
        s.sanitize();
        assert_eq!(s.zoom, 3.0);
        assert_eq!(s.tab_width, 4);
        assert_eq!(s.sidebar_width, 140.0);
        assert_eq!(s.window, None, "너무 작은 창 크기는 버림");
        assert_eq!(s.recent_files.len(), MAX_RECENT_FILES);
    }

    #[test]
    fn recent_files_are_unique_most_recent_first_and_capped() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        let file = |name: &str| {
            let p = dir.path().join(name);
            std::fs::write(&p, "").unwrap();
            p
        };
        let a = file("a.txt");
        let b = file("b.txt");
        s.add_recent(&a);
        s.add_recent(&b);
        s.add_recent(&a);
        assert_eq!(s.recent_files, [a.clone(), b.clone()]);
        // 같은 파일을 다른 경로 표기로 열어도 한 번만
        s.add_recent(&dir.path().join(".").join("b.txt"));
        assert_eq!(s.recent_files.len(), 2);
        for i in 0..15 {
            s.add_recent(&file(&format!("f{i}.txt")));
        }
        assert_eq!(s.recent_files.len(), MAX_RECENT_FILES);
        assert!(s.recent_files[0].ends_with("f14.txt"));
    }
}
