//! P1-01 통합 테스트: `--test-panic` 실행 시 로그 파일에 panic 정보가 기록되는지 검증한다.

use std::process::Command;

#[test]
fn test_panic_flag_writes_panic_info_to_log_file() {
    let exe = env!("CARGO_BIN_EXE_my-notepad");
    let log_path =
        std::env::temp_dir().join(format!("my-notepad-panic-test-{}.log", std::process::id()));
    let _ = std::fs::remove_file(&log_path);

    let status = Command::new(exe)
        .arg("--log-file")
        .arg(&log_path)
        .arg("--test-panic")
        .status()
        .expect("테스트용 바이너리 실행 성공해야 함");

    assert!(!status.success(), "--test-panic 실행은 비정상 종료여야 함");

    let log_contents = std::fs::read_to_string(&log_path).expect("panic 로그 파일이 생성되어야 함");
    assert!(
        log_contents.contains("PANIC"),
        "로그에 PANIC 태그가 포함되어야 함: {log_contents}"
    );
    assert!(
        log_contents.contains("P1-01 의도적 테스트 panic 발생"),
        "로그에 panic 메시지가 기록되어야 함: {log_contents}"
    );

    let _ = std::fs::remove_file(&log_path);
}
