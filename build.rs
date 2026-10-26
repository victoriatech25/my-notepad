//! Windows 실행 파일 리소스: 아이콘·버전 정보·제품명 (P5-01).
//!
//! 탐색기 아이콘과 "속성 → 자세히"의 제품명·파일 설명·버전을 넣는다. 버전은 `Cargo.toml`의 `version`을 따른다.
//! 다른 OS에서는 아무것도 하지 않는다 (macOS 아이콘·이름은 `.app` 번들의 `Info.plist`가 맡는다, P5-03).

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/icons/vic-notepad.ico");
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icons/vic-notepad.ico")
            .set("ProductName", "Vic Notepad")
            .set("FileDescription", "Vic Notepad")
            .set("InternalName", "my-notepad")
            .set("OriginalFilename", "my-notepad.exe");
        if let Err(e) = res.compile() {
            panic!("Windows 리소스 컴파일 실패: {e}");
        }
    }
}
