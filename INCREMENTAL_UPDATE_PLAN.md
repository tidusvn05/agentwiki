# Incremental update — kế hoạch & phân tích đánh đổi

Bối cảnh: khi chỉ đổi vài file, `agentwiki` hiện vẫn duyệt lại toàn bộ DAG.
Cache content-hash (`sha256(prompt‖model‖backend‖SCHEMA_VERSION)`,
`src/cache.rs`) làm cho các call không đổi prompt thành hit miễn phí —
nhưng chỉ ở **tầng leaf** (`dir_summary@<dir>`). Các agent tổng hợp toàn cục
(relationships, system_context, domain_modules, architecture, workflow,
boundary) và compose embed **toàn bộ dossier** nên hầu như luôn miss khi
code đổi → thực tế "đổi vài file" vẫn tốn ~20-40 calls.

Số liệu tham chiếu (repo này, run en 2026-09-20): ~62 calls =
`dir_summary` ~23 + global research ~10 + `deep_dive` ~7 + `key_module` ~3
+ compose ~15. Leaf fan-out đã incremental sẵn; phần luôn chạy lại là
tầng synthesis.

## Ba vấn đề thật cần giải

1. **Global prompts volatile** — 1 byte đổi trong dossier bust toàn bộ
   synthesis agents.
2. **Không phân biệt "đổi nhỏ" vs "đổi cấu trúc"** — sửa typo và thêm
   module mới được đối xử như nhau.
3. **Không có khái niệm "đủ mới"** — pipeline chỉ biết chạy lại, không
   biết docs hiện tại còn tươi không (staleness vô hình).

## Các góc nhìn đã xét trước khi chốt thiết kế

**Build-system analogy (Bazel/content-addressed).** Cache hiện tại vốn đã
là content-addressed store — phần thiếu không phải scheduler mà là *tầng
đo lường thay đổi*. Metric tối ưu là **số LLM call** (quota + latency),
không phải CPU scheduling — nên không cần incremental DAG scheduler thật.

**Data vs Prose — góc quyết định.** Có 2 lớp artifact: `research.json`
(data — factual, schema-validated, **checkable bằng `drift`**) và docs
(prose — narrative, decay khi bị patch). Patch prose là sai lầm của
hướng "patch-mode" kinh điển: mỗi patch đúng 95% nhưng sau 20 patch
docs còn ~1/3 mạch nguyên bản — hỏng kiểu không-gãy, reader không report
được. Đảo lại: **patch data, regenerate prose**. Delta vào research JSON
verify được; docs luôn viết lại từ data đầy đủ → không decay.

**Doc tree đã phân hoạch sẵn.** `4.Deep-Exploration/<Domain>.md` vốn
per-domain — phần lớn doc surface đã align với unit thay đổi. Chỉ
Overview/Architecture/Workflow là global.

**`drift` = safety net duy nhất.** Import extractor của drift đo được
**change significance không cần LLM**: đổi import graph (edge/dir mới) =
structural; chỉ đổi nội dung hàm = cosmetic. Đây là proxy tốt nhất cho
"kiến trúc có thật sự đổi không" — và là thứ khiến incremental *an toàn*
ở agentwiki mà tool khác không có được.

## Giải pháp — incremental mode

```
scan → diff manifest (hash per-dir, lưu trong state.json)
     → classify: cosmetic | structural
        cosmetic  (không dir mới/xóa, import graph nguyên)
          → reuse research.json → selective compose → ~5-10 calls
        structural (dir mới/xóa, import edges đổi)
          → delta research: global agents nhận
            {previous json + per-dir delta}, trả JSON HOÀN CHỈNH
            (không trả diff — tránh merge bug)
          → domain map chỉ re-derive khi structural
          → deep_dive chỉ cho domain có input đổi
          → compose full
     → drift verify claims mới (safety net)
     → agentwiki status: "docs tươi / N dir đã đổi / research cũ X commits"
```

Điểm mấu chốt: thay vài file → research nhận delta nhỏ (prompt nhỏ =
rẻ hơn) hoặc skip hẳn khi cosmetic → tổng ~5-15 calls thay ~40.

### Các mảnh triển khai cụ thể

1. **Manifest** — `state.json` lưu per-dir content hash + per-file hash;
   diff ở đầu pipeline. Nền tảng của mọi thứ còn lại, rẻ nhất, làm trước.
2. **`agentwiki status`** — báo độ tươi của docs (dirs đổi, commits kể từ
   lần research cuối). Có thể gộp vào `doctor`. Cho user quyết định có
   chủ đích thay vì chạy mò.
3. **Significance classifier** — deterministic: dir count delta +
   import-graph edge delta (tái dùng `drift::imports`/`drift::graph`).
   **Fail-open về structural**: nghi ngờ thì chạy full, không chạy thiếu.
4. **Delta-research prompts** — global agents nhận `{previous_output,
   delta}` thay vì full dossier khi delta nhỏ; vẫn yêu cầu trả JSON đầy
   đủ qua schema validation hiện có.
5. **Selective compose** — deep-dive docs recompose theo domain delta;
   global docs recompose khi research delta chạm tới. Manifest doc→input
   phải **fail-open về phía recompose** khi không chắc quan hệ phụ thuộc.

## Đánh đổi — và mức độ nghiêm trọng

| Đánh đổi | Mức độ | Giảm thiểu |
|---|---|---|
| Delta-research bỏ sót hiệu ứng emergent (file nhỏ tạo cycle, thay đổi ý nghĩa kiến trúc toàn cục) | **Trung bình — risk chính** | Classifier dùng import-graph delta (bắt được structural) + `drift` verify sau run + khuyến nghị periodic full-regen |
| Compose selective sai manifest → doc stale lặng lẽ | Trung bình | Fail-open về recompose; status báo tuổi từng doc |
| Prompt delta khó đúng (LLM lười copy / drop trường) | Trung bình | Yêu cầu trả JSON đầy đủ + schema validation sẵn có bắt phần lớn |
| Manifest/state phức tạp hơn, cần migration | Nhỏ | `state.json` thêm field, versioned |
| Classifier phân sai hướng nguy hiểm (structural→cosmetic) | Nhỏ nếu fail-open | Cosmetic→structural chỉ phí calls; chiều ngược lại mới độc — thiết kế bias sang structural |
| Cache brittle với rename/move file | Vốn có, không đổi | Content-hash trung thực nhưng không "semantic" — chấp nhận |

**Đánh giá tổng**: đánh đổi không lớn nếu giữ đúng nguyên tắc —
*patch data, không patch prose; fail-open về phía regen; drift verify
sau mỗi incremental run; full-regen định kỳ làm reset*. Vấn đề duy nhất
đáng ngại là prose decay — và thiết kế này tránh nó bằng cách không bao
giờ patch prose.

## Ràng buộc

- Giữ `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` sạch.
- Test offline (MockBackend); không cần CLI thật cho unit/integration.
- Full-regen path hiện tại phải giữ nguyên semantics — incremental là
  opt-in (`--incremental` hoặc config), mặc định vẫn full.
- `drift` chạy sau incremental run như verify tầng cuối.

## Thứ tự đề xuất

1 → 2 → 3 (mỗi mảnh độc lập ship được; 1+2 đã đem lại visibility).
4 → 5 khi đã có manifest ổn định.

## Không làm — và lý do

- **Patch prose trực tiếp** — decay tích lũy vô hình (xem góc nhìn
  Data vs Prose).
- **Incremental DAG scheduler thật** (input-manifest per agent,
  dependency-aware scheduling) — cache content-hash đã làm việc tương
  đương ở mức call; phần thêm chỉ là tiết kiệm vài giây render, đổi lấy
  phức tạp scheduler + risk miss dep → stale âm thầm.
- **Regen mỗi commit trong CI** — `drift --strict` đã cover khoảng giữa
  hai lần regen; docs không cần tươi tới từng commit.
- **Semantic/embedding-based change detection** — overkill; import-graph
  delta là proxy đủ tốt và deterministic.
