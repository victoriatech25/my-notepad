# 파일·폴더 아이콘

PLAN.md §3.1-3. P0-UX2(2026-10-20)에서 **A안: 단색 아이콘 폰트 + 파일 종류별 색**을 채택했다. 비교했던 B안(Material 다색 SVG)은 삭제했다.

| 폴더 | 내용 | 출처 | 라이선스 |
| :--- | :--- | :--- | :--- |
| `phosphor/` | `Phosphor.ttf` (Regular) | Phosphor Icons, egui-phosphor 0.14.0 크레이트에 포함된 파일 | MIT (`phosphor/LICENSE`) |

아이콘 결정 규칙(파일 이름 → 확장자 → 기본, 폴더 이름별 색 + 열림/닫힘)과 색은 지금 `spikes/ux-prototype/src/icons.rs`에 있다. P2-05에서 `src/core/file_icons.rs`와 매핑 표 `icons.toml`로 옮긴다.
