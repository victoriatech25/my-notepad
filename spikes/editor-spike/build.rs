//! 화면에 표시할 egui 버전을 Cargo.lock에서 읽어 컴파일 타임 상수로 넘긴다.

use std::path::Path;

fn main() {
    let lock_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock_path.display());

    let version = std::fs::read_to_string(&lock_path)
        .ok()
        .and_then(|lock| egui_version(&lock))
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=EGUI_VERSION={version}");
}

fn egui_version(lock: &str) -> Option<String> {
    let mut lines = lock.lines();
    while let Some(line) = lines.next() {
        if line.trim() == "name = \"egui\"" {
            let version = lines.next()?.trim().strip_prefix("version = \"")?;
            return Some(version.trim_end_matches('"').to_owned());
        }
    }
    None
}
