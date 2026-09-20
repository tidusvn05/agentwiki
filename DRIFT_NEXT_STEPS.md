# `agentwiki drift` — vòng tiếp theo: đóng vòng lặp & độ phủ

Tiếp nối commit `bf440e1`. Tính năng `drift` đã chạy đúng và đã review xong
(0 phantom / 0 reversed trên repo này, phantom & reversed gài vào đều bị bắt).

Vòng này **không thêm sức kiểm tra**. Nó giải quyết hai điểm yếu còn lại:
tool dễ bị bỏ không chạy, và khi chạy thì không phân biệt được "sạch" với
"chưa kiểm". Ba mục dưới đây nhỏ và liên quan nhau, làm trong cùng một vòng.

Ràng buộc chung: giữ `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`
sạch; test không giảm (hiện 86 pass); `cargo run -- drift` trên repo này phải
**vẫn 0 phantom, 0 reversed**.

---

## 1. Pipeline tự xuất claims vào thư mục output

**Vấn đề.** `.agentwiki/` bị gitignore, nên muốn chạy `drift` trên CI phải tự nhớ
`agentwiki drift --export-claims <file>` rồi commit file đó. Quên một lần là CI
im lặng vĩnh viễn mà không ai biết. Đây là rủi ro lớn hơn false positive: tool
đúng nhưng không được chạy.

**Sửa.** Cuối pipeline, ghi claims vào **thư mục output** (nơi docs đã được commit
— repo này có 30 file dưới `docs/`). Ai commit docs là tự động có claims.

- Hook tại `src/pipeline/mod.rs:223-226`, ngay cạnh `verify` + `write_summary`,
  trong cùng nhánh `if !pctx.config.skip_documentation`.
- Tên file: `<output_path>/agentwiki.claims.json`.
- Nội dung: đúng định dạng `--export-claims` đang sinh, để `--claims` đọc lại được
  không cần đổi gì. Tái dùng hàm export sẵn có trong `src/drift/claims.rs:74`
  thay vì viết lại — tách phần ghi thành hàm public nếu cần.
- Non-fatal: lỗi ghi chỉ `tracing::warn!`, không làm pipeline fail. Cùng tinh thần
  `verify` (`src/output/verify.rs:64`).
- Thêm `<output_path>/agentwiki.claims.json` vào thứ tự tìm claims mặc định trong
  `DriftConfig::claims_path()` (`src/drift/config.rs:137`), **sau** `<internal>/research.json`.
  Thứ tự cuối: `--claims` → `[drift].claims_path` → `<internal>/research.json` →
  `<output>/agentwiki.claims.json`.
- Cập nhật README: bỏ bớt phần hướng dẫn `--export-claims` thủ công ở giai đoạn 2
  của rollout, vì giờ chỉ cần commit docs. Giữ `--export-claims` cho người muốn
  đặt file ở chỗ khác.

**Verify.** Chạy pipeline với mock backend trên `tests/fixture-app`, xác nhận
`<output>/agentwiki.claims.json` xuất hiện, rồi `agentwiki drift` trên repo đó
chạy được khi đã xóa `.agentwiki/`. Thêm assert vào `tests/pipeline_offline.rs`
(test `full_pipeline_offline` đã có sẵn chỗ để kiểm file output).

## 2. In tỉ lệ độ phủ trong report

**Vấn đề.** Report nói `0 phantom` nhưng không nói 5/13 edge **chưa từng được kiểm**.
Người đọc không phân biệt được "docs khớp code" với "tool không kiểm nổi". Trên repo
này độ phủ thật là 62%.

**Sửa.** Thêm một dòng vào cả report người đọc (`src/drift/report.rs`) và
`drift.json`:

```
coverage: 8/13 claims checked (62%) — 5 unverifiable
```

- Mẫu số = tổng claim sau khi gộp trùng. Tử số = số claim có verdict
  `confirmed` / `phantom` / `reversed` (tức đã thực sự được đối chiếu).
  `structural` và `unverifiable` không tính là đã kiểm.
- Trong `drift.json` thêm field `coverage: { checked, total, ratio }` dưới `counts`.
- Với `-v`, gộp lý do `unverifiable` theo `reason` kèm số lượng (hiện đã liệt kê
  từng cái, chỉ cần thêm dòng tổng) để thấy ngay nguyên nhân phủ thấp nằm ở đâu.

**Verify.** Trên repo này phải ra đúng `8/13 (62%)`. Trên `tests/fixture-app`:
`5/6 (83%)`. Thêm assert vào `tests/drift_offline.rs`.

## 3. Định nghĩa lại `DataFlow` trong prompt

**Vấn đề.** Đây là nguyên nhân gốc của lỗ hổng độ phủ, và nó nằm ở prompt chứ không
ở code. `prompts/relationships.md` **chưa bao giờ định nghĩa DataFlow là gì**, trong
khi dòng 8 lại mời gọi nó ("Cross-directory module dependencies and data flows").
Dòng 23 chỉ nói "nếu không chắc dùng Module". Kết quả: 4/13 edge của repo này bị gán
DataFlow một cách tùy ý, và `drift` buộc phải xếp chúng vào `unverifiable` — vì dữ
liệu có thể chảy **ngược** chiều import (chính xác là trường hợp
`src/agent/reports -> src/output` gán DataFlow trong khi import đi chiều ngược lại),
nên kiểm bằng import graph sẽ sinh false positive.

**Sửa.** Trong `prompts/relationships.md`:

- Thu hẹp `DataFlow`: chỉ dùng khi giữa hai bên **không có tham chiếu ở mức code** —
  dữ liệu đi qua file dùng chung, database, message queue, hoặc process khác. Nếu có
  bất kỳ import / gọi hàm / dùng kiểu nào thì phải chọn
  `Import` / `FunctionCall` / `Composition` / `Module`.
- Sửa dòng 8 để không mời gọi DataFlow cho quan hệ module thông thường.
- Giữ dòng 23 (`Module` là lựa chọn khi không chắc) — `Module` vẫn kiểm được.

**Bắt buộc kèm theo.** Đổi prompt là đổi cache key → bump `SCHEMA_VERSION` trong
`src/cache.rs:13` từ `"1"` lên `"2"`, theo CONTRIBUTING. Nêu rõ trong commit message
rằng cache của user sẽ bị invalidate.

**Verify.** Chạy lại research trên repo này với backend thật rồi so: số edge
`kind_not_checkable` phải giảm (kỳ vọng phủ lên khoảng 90%), và **vẫn 0 phantom,
0 reversed**. Nếu phát sinh phantom, đó là tín hiệu thật cần xem — không được
che bằng cách nới bộ lọc.

Lưu ý: mục này phụ thuộc vào output của LLM nên không assert cứng trong test được.
Chỉ kiểm bằng tay và ghi kết quả vào commit message.

---

## Thứ tự

1 → 2 → 3. Mục 1 và 2 độc lập nhau, có thể làm song song. Mục 3 làm cuối để đo được
độ phủ trước/sau bằng dòng `coverage:` của mục 2.

## Ngoài phạm vi vòng này

- **Kiểm Mermaid trong docs so với claims.** Milestone riêng, đáng làm sau khi có số
  độ phủ thật. Bắt một lớp lỗi khác (docs tự mâu thuẫn), không cần import graph nên
  rủi ro false positive gần như 0.
- **`domain_relations`** — vẫn để v2 như plan gốc.
- **Thêm ngôn ngữ (Go, Java, C#)** — chỉ làm khi có repo thật cần, không xây sẵn.
- **Tách file > 400 dòng** (`graph.rs` 548, `compare.rs` 458, `imports/rust.rs` 444)
  — đã cân nhắc và bỏ qua, không có đường cắt tự nhiên.
