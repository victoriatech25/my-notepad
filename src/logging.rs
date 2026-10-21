//! 파일 로깅 및 panic hook 초기화 (PLAN.md §3.6, §6.2 · WORKPLAN.md P1-01).
//!
//! - 기본 로그 파일 위치:
//!   - Windows: `%APPDATA%\My-Notepad\logs\my-notepad.log`
//!   - macOS: `~/Library/Logs/My-Notepad/my-notepad.log`
//! - 테스트 및 진단용으로 `--log-file <경로>` 또는 `MY_NOTEPAD_LOG_FILE` 환경 변수로 경로를 지정할 수 있다.
//! - panic 발생 시 위치·스레드·메시지를 로그 파일에 즉시 기록하고 flush 한다.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use log::{Level, LevelFilter, Log, Metadata, Record};

struct FileLogger {
    file: Arc<Mutex<Option<File>>>,
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= Level::Info
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "[{} {:<5} {}] {}\n",
            timestamp_secs(),
            record.level(),
            record.target(),
            record.args()
        );
        write_and_flush(&self.file, &line);
    }

    fn flush(&self) {
        if let Ok(mut guard) = self.file.lock()
            && let Some(file) = guard.as_mut()
        {
            let _ = file.flush();
        }
    }
}

/// 로거와 panic hook을 초기화하고 실제 사용된 로그 파일 경로를 돌려준다.
pub fn init(custom_path: Option<PathBuf>) -> Option<PathBuf> {
    let path = custom_path
        .or_else(|| std::env::var_os("MY_NOTEPAD_LOG_FILE").map(PathBuf::from))
        .or_else(default_log_path)?;

    let file = open_log_file(&path);
    let shared = Arc::new(Mutex::new(file));

    let logger = Box::new(FileLogger {
        file: Arc::clone(&shared),
    });
    if log::set_boxed_logger(logger).is_ok() {
        log::set_max_level(LevelFilter::Info);
    }

    install_panic_hook(shared, path.clone());
    Some(path)
}

fn open_log_file(path: &Path) -> Option<File> {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn install_panic_hook(file: Arc<Mutex<Option<File>>>, log_path: PathBuf) {
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let entry = format_panic(info);
        write_and_flush(&file, &entry);
        // 로거 파일 핸들이 닫혔거나 잠금에 실패한 경우에도 직접 열어 기록을 시도한다.
        if file.lock().map(|g| g.is_none()).unwrap_or(true)
            && let Some(mut f) = open_log_file(&log_path)
        {
            let _ = f.write_all(entry.as_bytes());
            let _ = f.flush();
        }
        prev_hook(info);
    }));
}

fn write_and_flush(file: &Arc<Mutex<Option<File>>>, text: &str) {
    if let Ok(mut guard) = file.lock()
        && let Some(f) = guard.as_mut()
    {
        let _ = f.write_all(text.as_bytes());
        let _ = f.flush();
    }
}

/// panic 정보를 로그 한 줄로 포맷한다.
pub fn format_panic(info: &PanicHookInfo<'_>) -> String {
    let thread = std::thread::current();
    let thread_name = thread.name().unwrap_or("unnamed");
    let location = info.location().map_or_else(
        || "unknown:0:0".to_owned(),
        |loc| format!("{}:{}:{}", loc.file(), loc.line(), loc.column()),
    );
    let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "알 수 없는 panic".to_owned()
    };
    format!(
        "[{} PANIC thread='{}' at {}] {}\n",
        timestamp_secs(),
        thread_name,
        location,
        payload
    )
}

/// OS별 기본 로그 파일 경로를 반환한다 (PLAN.md §6.2).
pub fn default_log_path() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).or_else(|| {
            std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("AppData/Roaming"))
        })?;
        Some(base.join("My-Notepad").join("logs").join("my-notepad.log"))
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        Some(
            home.join("Library")
                .join("Logs")
                .join("My-Notepad")
                .join("my-notepad.log"),
        )
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        Some(
            home.join(".local")
                .join("state")
                .join("my-notepad")
                .join("logs")
                .join("my-notepad.log"),
        )
    }
}

fn timestamp_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_log_path_ends_with_expected_file() {
        let path = default_log_path().expect("기본 로그 경로를 구할 수 있어야 함");
        assert_eq!(
            path.file_name().and_then(|s| s.to_str()),
            Some("my-notepad.log")
        );
    }

    #[test]
    fn open_and_write_log_file() {
        let path =
            std::env::temp_dir().join(format!("my-notepad-log-unit-{}.log", std::process::id()));
        let _ = fs::remove_file(&path);

        let file = open_log_file(&path);
        assert!(file.is_some());
        let shared = Arc::new(Mutex::new(file));
        write_and_flush(&shared, "[TEST] log write ok\n");

        let content = fs::read_to_string(&path).expect("로그 파일 읽기 성공");
        assert!(content.contains("[TEST] log write ok"));
        let _ = fs::remove_file(&path);
    }
}
