# 파일·폴더 아이콘

PLAN.md §3.1-3. P0-UX2(2026-10-20)에서 **A안: 단색 아이콘 폰트 + 파일 종류별 색**을 채택했다. 비교했던 B안(Material 다색 SVG)은 삭제했다.

| 파일 | 내용 | 출처 | 라이선스 |
| :--- | :--- | :--- | :--- |
| `phosphor/Phosphor.ttf` | 아이콘 폰트 (Regular) | Phosphor Icons, egui-phosphor 0.14.0 크레이트에 포함된 파일 | MIT (`phosphor/LICENSE`) |
| `icons.toml` | 매핑 표: 글리프 이름·색, 파일 이름 → 확장자 → 기본, 폴더 열림/닫힘·이름별 색 | 이 프로젝트 | 프로젝트와 같음 |

`icons.toml`은 실행 파일에 내장되어 `src/core/file_icons.rs`가 처음 쓸 때 읽는다 (P2-05). 아이콘을 추가할 때는 `[glyphs]`에 Phosphor 코드포인트를 적고 `[files.names]`·`[files.extensions]`·`[folders.names]`에서 그 이름을 쓴다. 표에 적은 글리프가 폰트에 실제로 있는지는 `cargo test`가 검사한다.
