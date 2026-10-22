//! 명령줄 인자 해석 (PLAN.md §3.1, P1-13).
//!
//! - `my-notepad a.txt b.txt` → 파일마다 탭으로 연다. 탐색기 "연결 프로그램"도 같은 방식으로 경로 하나를 넘긴다.
//! - 상대 경로는 실행한 폴더 기준 절대 경로로 바꾼다 (탭 툴팁·중복 열기 판정에 전체 경로가 필요).
//! - `--` 뒤의 인자는 모두 파일 경로로 본다 (`--`로 시작하는 파일 이름도 열 수 있게).
//! - 개발·검증용 플래그: `--test-panic`, `--font-report`, `--log-file <경로>` / `--log-file=<경로>`.
//!   알 수 없는 `--` 플래그는 무시하고 [`CliArgs::unknown`]에 남긴다.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// 해석된 명령줄 인자.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CliArgs {
    pub log_file: Option<PathBuf>,
    pub test_panic: bool,
    pub font_report: bool,
    /// 열 파일 (절대 경로, 받은 순서)
    pub files: Vec<PathBuf>,
    /// 알 수 없는 플래그 (로그에 경고만 남긴다)
    pub unknown: Vec<String>,
}

impl CliArgs {
    /// 현재 프로세스의 인자를 해석한다.
    pub fn from_env() -> Self {
        let cwd = std::env::current_dir().unwrap_or_default();
        Self::parse(std::env::args_os().skip(1), &cwd)
    }

    /// 프로그램 이름을 뺀 인자 목록을 `cwd` 기준으로 해석한다.
    pub fn parse(args: impl IntoIterator<Item = OsString>, cwd: &Path) -> Self {
        let mut cli = Self::default();
        let mut args = args.into_iter();
        let mut only_files = false;

        while let Some(arg) = args.next() {
            if only_files {
                cli.files.push(absolute(cwd, PathBuf::from(arg)));
                continue;
            }
            // 플래그는 모두 ASCII이므로, UTF-8이 아닌 인자는 파일 경로로만 쓴다.
            let Some(text) = arg.to_str() else {
                cli.files.push(absolute(cwd, PathBuf::from(arg)));
                continue;
            };
            match text {
                "--" => only_files = true,
                "--test-panic" => cli.test_panic = true,
                "--font-report" => cli.font_report = true,
                "--log-file" => cli.log_file = args.next().map(PathBuf::from),
                _ if text.starts_with("--log-file=") => {
                    cli.log_file = Some(PathBuf::from(&text["--log-file=".len()..]));
                }
                _ if text.starts_with("--") => cli.unknown.push(text.to_owned()),
                _ => cli.files.push(absolute(cwd, PathBuf::from(arg))),
            }
        }
        cli
    }
}

fn absolute(cwd: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> CliArgs {
        let cwd = std::env::temp_dir().join("work");
        CliArgs::parse(args.iter().map(OsString::from), &cwd)
    }

    #[test]
    fn files_become_absolute_in_order() {
        let cwd = std::env::temp_dir().join("work");
        let abs = std::env::temp_dir().join("메모.txt");
        let cli = parse(&["a.txt", "하위/b.md", abs.to_str().unwrap()]);
        assert_eq!(cli.files, [cwd.join("a.txt"), cwd.join("하위/b.md"), abs]);
        assert!(cli.unknown.is_empty());
    }

    #[test]
    fn flags_are_separated_from_files() {
        let cli = parse(&[
            "--log-file",
            "x.log",
            "--font-report",
            "--verbose",
            "note.txt",
            "--log-file=y.log",
        ]);
        assert_eq!(cli.log_file, Some(PathBuf::from("y.log")));
        assert!(cli.font_report);
        assert!(!cli.test_panic);
        assert_eq!(cli.unknown, ["--verbose"]);
        assert_eq!(cli.files.len(), 1);
        assert!(cli.files[0].ends_with("note.txt"));
    }

    #[test]
    fn double_dash_makes_rest_files() {
        let cli = parse(&["--", "--weird.txt", "--test-panic"]);
        assert!(!cli.test_panic);
        assert_eq!(cli.files.len(), 2);
        assert!(cli.files[0].ends_with("--weird.txt"));
    }
}
