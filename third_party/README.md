# third_party — 패치한 외부 크레이트 사본

루트 `Cargo.toml`의 `[patch.crates-io]`로 crates.io 배포판 대신 쓰는 사본이다. 워크스페이스 멤버가 아니므로(`exclude`) 우리 fmt·clippy 규칙을 적용하지 않는다.

## winit-0.30.13

| 항목 | 내용 |
| :--- | :--- |
| 원본 | crates.io `winit` 0.30.13 (eframe/egui-winit 0.37.0이 쓰는 버전) 그대로 복사 |
| 결함 | macOS Apple 한글 2벌식에서 앱 첫 입력이나 입력 소스 전환 직후 **첫 키가 조합(preedit) 없이 낱자로 확정**된다. 예: `abc` → 한/영 → `ㄱ` `ㅣ` `ㅁ` 입력 시 `abc김`이 아니라 `abcㄱㅣ`… ([winit#3095](https://github.com/rust-windowing/winit/issues/3095)) |
| 비교 | 같은 Mac에서 메모·텍스트 편집기는 정상 조합 (2026-10-23, P1-MAC 1-4g) |
| 수정 | upstream PR [winit#4744](https://github.com/rust-windowing/winit/pull/4744)를 0.30 구조에 맞춰 백포트. 첫 키가 정확히 낱자 한 글자의 `insertText`로만 처리되면 같은 키 이벤트를 IME에 한 번 더 해석시킨다 |
| 바뀐 파일 | `src/platform_impl/macos/ime_startup.rs`(신규, 판정 로직 + 테스트 7개), `src/platform_impl/macos/mod.rs`, `src/platform_impl/macos/view.rs` |
| 0.30 대응 | master의 `ime_capabilities`는 0.30의 `ime_allowed`, `enable_ime`/`disable_ime` 초기화는 `set_ime_allowed`, `ModifiersState::META`는 `SUPER`로 옮겼다. 0.30에는 `pending_commit`이 없어 그 조건은 뺐다 (0.30은 키 처리 안에서 `Committed` 상태를 바로 정리한다) |
| 제거 조건 | eframe이 쓰는 winit 배포판에 #4744(또는 같은 수정)가 들어오면 사본과 `[patch.crates-io]`를 지운다 |

원본과의 차이는 다음으로 확인한다.

```bash
diff -ru ~/.cargo/registry/src/index.crates.io-*/winit-0.30.13 third_party/winit-0.30.13
```
