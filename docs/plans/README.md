# Plans

Kế hoạch thiết kế của agentwiki. Mỗi file mở đầu bằng dòng `Status:`.

Thư mục này **nằm trong `scan.excluded_dirs`** (qua `docs`) nên không lọt vào
materials khi agentwiki generate docs cho chính nó — xem ghi chú ở
`agentwiki.toml`.

## Active

| Plan | Nội dung |
|---|---|
| [`active/incremental-update.md`](active/incremental-update.md) | Chỉ chạy lại phần đã đổi thay vì toàn bộ DAG. Chưa implement mảnh nào. |

## Done

| Plan | Nội dung | Kết quả |
|---|---|---|
| [`done/2025-implementation-v1.md`](done/2025-implementation-v1.md) | Plan xây dựng v1 (M0–M6): backend trait, task DAG, output tree | Ship ở v0.5.0. **Đã lệch khỏi code**, chỉ đọc làm lịch sử |
| [`done/drift-check.md`](done/drift-check.md) | `agentwiki drift`: so claims vs import graph, chuỗi lọc C1–C12 / U1–U9 | Toàn bộ `src/drift/**` |
| [`done/drift-next-steps.md`](done/drift-next-steps.md) | Đóng vòng lặp: pipeline tự xuất claims, dòng `coverage:`, siết `DataFlow` | Cả 3 mục xong |

## Quy ước

- Kiến trúc **hiện tại** không nằm ở đây. Xem
  [`docs/en/2.Architecture.md`](../en/2.Architecture.md) — generated và verify
  được bằng `agentwiki drift`. Plan là *ý định tại một thời điểm*, không phải
  mô tả hệ thống.
- Plan xong → chuyển sang `done/`, thêm `Status: hoàn thành` kèm con trỏ tới
  code đã ship. Không xoá: lý do thiết kế không suy ra được từ code.
- Plan trong `done/` **không được cập nhật** cho khớp code. Nếu nó đã lệch,
  ghi rõ lệch chỗ nào trong header thay vì sửa nội dung.
