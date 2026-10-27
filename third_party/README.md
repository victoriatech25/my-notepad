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
| 제거 조건 | eframe이 쓰는 winit 배포판에 #4744(또는 같은 수정)가 들어오면 사본과 `[patch.crates-io]`를 지운다. 단 아래 수정 2~5가 upstream에 없으면 그것만 남긴 사본을 유지하거나 앱 쪽 대안을 다시 찾는다 |

### 수정 2 — 조합 중 `Cmd`/`Ctrl` 단축키 전달 (자체 수정, upstream 없음)

| 항목 | 내용 |
| :--- | :--- |
| 결함 | 한글 조합 중 `Cmd+K`·`Cmd+S` 같은 단축키를 누르면 입력기가 조합만 확정(`Commit`)하고 winit이 키 이벤트를 앱에 보내지 않는다. 그래서 단축키가 실행되지 않는다 (P1-MAC 1-5c: `Cmd+K` 다음 `V`가 `ㅍ`로 입력됨) |
| 확인 | 본 앱 임시 로그: `Key(SuperLeft)` → `Preedit "마"` → `Commit("마")`, `K [Cmd]` 키 이벤트 없음 |
| 기준 | macOS 텍스트 편집기는 조합 중 `Cmd+S`를 누르면 조합을 확정하고 저장한다 |
| 수정 | `view.rs` `keyDown`: `Cmd`·`Ctrl`이 눌린 키는 입력기가 조합을 확정했어도 `KeyboardInput`을 함께 보낸다. `Cmd` 조합은 egui-winit이 글자로 만들지 않으므로 단축키로만 처리된다 |

### 수정 3 — 조합 끊기 시 입력기 조합도 버림 (자체 수정, upstream 없음)

| 항목 | 내용 |
| :--- | :--- |
| 결함 | 앱이 조합을 끊으면(egui `interrupt_ime` → `set_ime_allowed(false)`) winit은 자기 쪽 조합 텍스트만 비우고 입력기에는 알리지 않는다. 입력기 안에 남은 `ㅍ`가 다음 키와 합쳐져 `파`가 된다 (`Cmd+K` 다음 `V` 후 `k`) |
| 수정 | `view.rs` `set_ime_allowed(false)`: 조합 텍스트가 있으면 `NSTextInputContext::discardMarkedText()`를 호출한다 (`unmarkText`가 이미 쓰는 방식) |

### 수정 4 — `Cmd+Q`·Dock 종료를 창 닫기 요청으로 바꿈 (자체 수정, upstream 없음)

| 항목 | 내용 |
| :--- | :--- |
| 결함 | `Cmd+Q`(winit 기본 앱 메뉴 Quit)·Dock 우클릭 종료·로그아웃은 `NSApplication terminate:`로 가는데, 앱 델리게이트에 `applicationShouldTerminate:`가 없어 macOS가 즉시 종료한다. 저장하지 않은 문서 확인을 건너뛴다 (P1-MAC 1-6a·c, P1-11 🍎). 빨간 닫기 버튼은 `windowShouldClose:` → `CloseRequested`라 가로채진다 |
| 기준 | macOS 텍스트 편집기는 저장하지 않은 문서가 있으면 `Cmd+Q`·Dock 종료에서 확인을 받는다 |
| 수정 | `app_state.rs`: `applicationShouldTerminate:` 구현. winit 창이 있으면 창마다 `CloseRequested`를 보내고(빨간 버튼과 같은 경로) `NSTerminateCancel`을 돌려준다. winit 창이 없으면 `NSTerminateNow`. 앱이 종료를 승인하면 이벤트 루프는 `stop:`으로 끝나므로 `terminate:`를 다시 거치지 않는다 |

### 수정 5 — Finder에서 연 파일 받기 (자체 수정, upstream 없음)

| 항목 | 내용 |
| :--- | :--- |
| 결함 | Finder "다음으로 열기"·연결된 문서 더블클릭·Dock 아이콘에 끌어 놓기·`open -a`는 실행 인자가 아니라 `odoc` Apple 이벤트로 온다. AppKit은 이것을 앱 델리게이트의 `application:openURLs:`로 넘기는데 winit 델리게이트에 이 메서드가 없고 winit 0.30에는 대응하는 이벤트도 없다. 그래서 파일이 열리지 않고 macOS가 "열 수 없음" 경고를 띄운다 (P5-03) |
| 기준 | macOS 텍스트 편집기는 Finder에서 연 파일을 (이미 떠 있으면 그 앱에서) 연다 |
| 수정 | `app_state.rs`: `application:openURLs:` 구현 — 파일 URL만 경로로 바꿔 메인 스레드의 처리기에 넘긴다. `platform/macos.rs`: 처리기를 등록하는 `set_open_files_handler` 추가(`platform_impl/macos/mod.rs`에서 내보냄). `Cargo.toml`: `objc2-foundation`에 `NSURL` 기능 추가. 앱을 실행시킨 파일은 `applicationDidFinishLaunching:`보다 먼저 오므로 앱은 이벤트 루프 전에 등록하고 받은 경로를 쌓아 둔다(`src/single_instance.rs`) |
| 제거 조건 | winit 0.31 이상은 이 이벤트를 다루는 방식이 다르다. eframe이 쓰는 winit 배포판이 파일 열기 이벤트를 제공하면 그것으로 옮긴다 |

원본과의 차이는 다음으로 확인한다.

```bash
diff -ru ~/.cargo/registry/src/index.crates.io-*/winit-0.30.13 third_party/winit-0.30.13
```
