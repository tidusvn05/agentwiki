# Incremental update — kế hoạch & phân tích đánh đổi

> **Status: implemented (mảnh 4 deferred).** Mảnh 0/1/2/3 + phần fail-open
> của mảnh 5 đã ship; delta-research giữ nguyên gate "chỉ làm sau khi đo".
> Last updated: 2026-09-21.

Bối cảnh: khi chỉ đổi vài file, `agentwiki` hiện vẫn duyệt lại toàn bộ DAG.
Cache content-hash (`sha256(prompt‖model‖backend‖SCHEMA_VERSION)`,
`src/cache.rs`) làm cho các call không đổi prompt thành hit miễn phí —
nhưng chỉ ở **tầng leaf** (`dir_summary@<dir>`). Mọi node phía trên nhận
nguyên vẹn output của dep qua `build_materials` (`src/agent/runner.rs:321`)
nên hầu như luôn miss khi code đổi → "đổi vài file" vẫn tốn ~25 calls.

## Số liệu tham chiếu

Run `en` trên chính repo này, 2026-09-20
(`docs/en/__AgentWiki_Summary__.md`): **48 CLI calls, 1325.5s**.

| Nhóm | Calls | Thời gian | Fan-out | Luôn rerun? |
|---|---|---|---|---|
| `dir_summary` | 24 | 435.7s | `PerDir` | chỉ dir đã đổi |
| global research (7 node) | 7 | 475.4s | — | **có** |
| `key_module` | 7 | 221.8s | `PerDomain` | **có** |
| `deep_dive` | 7 | 441.2s | `PerDomain` | **có** |
| compose LLM (`overview`, `architecture_doc`, `workflow_doc`) | 3 | 243.5s | — | **có** |
| `boundary_doc`, `database_doc` | 0 | 0.0s | `ExecKind::Deterministic` | — |

global research = `system_context`, `relationships`, `database`, `boundary`,
`domain_modules`, `architecture`, `workflow`.

Đọc bảng này ra hai kết luận định hướng cả plan:

1. Phần luôn-rerun là **24/48 call** — đúng một nửa.
2. Trong 24 call đó, **14 call (`key_module` + `deep_dive`, 663s = 50% wall
   time) đã là `PerDomain`** — tức đã phân hoạch sẵn theo unit thay đổi.
   Chúng rerun không phải vì thiếu manifest, mà vì prompt của chúng nhận
   thừa input. Đây là chỗ rẻ nhất để lấy được lợi ích lớn nhất.

## Ba vấn đề thật cần giải

1. **Input của node fan-out quá rộng** — mỗi instance `PerDomain` nhận toàn
   bộ output dep, nên 1 byte đổi ở bất kỳ domain nào bust cả 14 call.
2. **Không phân biệt "đổi nhỏ" vs "đổi cấu trúc"** — sửa typo và thêm
   module mới được đối xử như nhau.
3. **Không có khái niệm "đủ mới"** — pipeline chỉ biết chạy lại, không
   biết docs hiện tại còn tươi không (staleness vô hình).

Và một bug có sẵn, phải sửa cùng lúc vì cùng cơ chế:

4. **`mode = "agentic"` cache sai chiều.** `build_materials` return sớm
   (`runner.rs:327`) → code không vào prompt → sửa code không đổi cache key
   → **cache hit trả docs cũ, không báo gì**. Ở mode này cache đang
   *under*-invalidate, ngược hẳn với `embedded`. Mọi thiết kế dưới đây phải
   đúng cho cả hai mode.

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

**Thu hẹp input trước khi thêm cơ chế.** Trước khi thêm manifest (state mới,
có thể sai, gây stale âm thầm), tận dụng cache đã có bằng cách cho prompt
nhận ít input hơn. Thu hẹp input là thay đổi cục bộ, review được, không
thêm state; manifest là cơ chế *bỏ qua* call, sai thì hỏng im lặng. Làm
cái rẻ và an toàn trước, đo, rồi mới quyết cái đắt.

**Doc tree đã phân hoạch sẵn.** `4.Deep-Exploration/<Domain>.md` vốn
per-domain — phần lớn doc surface đã align với unit thay đổi. Chỉ
Overview/Architecture/Workflow là global.

**`drift` = safety net duy nhất.** Import extractor của drift đo được
**change significance không cần LLM**: đổi import graph (edge/dir mới) =
structural; chỉ đổi nội dung hàm = cosmetic. Đây là proxy tốt nhất cho
"kiến trúc có thật sự đổi không" — và là thứ khiến incremental *an toàn*
ở agentwiki mà tool khác không có được.

## Nguyên tắc bất biến

Ba điều này ràng buộc mọi mảnh bên dưới; vi phạm điều nào thì mảnh đó sai.

- **Manifest key ⊇ cache key.** Bất kỳ đường tắt nào bỏ qua cache đều phải
  tự gánh lại *toàn bộ* input mà cache key đang bảo vệ. Xem mảnh 1.
- **Fail-open về phía regenerate.** Không chắc → chạy lại. Phân sai
  cosmetic→structural chỉ tốn call; chiều ngược lại cho ra docs sai.
- **Incremental run ≡ full run trên cùng trạng thái repo.** Với MockBackend
  deterministic, hai đường phải cho output giống hệt nhau. Đây là invariant
  test được, và là thứ duy nhất chặn stale âm thầm. Xem "Chiến lược test".

## Giải pháp — incremental mode

```
scan → fingerprint per-file/per-dir → diff manifest
     → classify: cosmetic | structural
        cosmetic  (không dir mới/xóa, import graph nguyên)
          → no-op: 0 call, docs không đổi, in lý do
        structural (dir mới/xóa, import edges đổi)
          → dir_summary chỉ cho dir đã đổi        (cache lo, đã có)
          → key_module/deep_dive chỉ cho domain có input đổi  (mảnh 0)
          → global research + compose chạy lại     (mặc định)
     → drift verify claims mới (safety net)
     → agentwiki status: "docs tươi / N dir đã đổi / research cũ X commits"
```

### Mảnh 0 — Thu hẹp input của fan-out `PerDomain` *(làm trước tiên)*

**Vấn đề.** `build_materials` (`runner.rs:318-330`) nhét nguyên vẹn output
của mọi dep vào prompt:

```rust
for dep in spec.deps {
    if let Some(v) = pctx.ctx.get(dep).await {
        s.push_str(&materials::dep_block(registry::display_name(dep), &v));
    }
}
```

`key_module` có `deps = ["system_context", "domain_modules"]` → mỗi instance
per-domain nhận **toàn bộ** `domain_modules` JSON (cả 7 domain). Đổi mô tả
domain A → prompt của cả 7 instance đổi → 7 call. `deep_dive` tương tự.

Đáng chú ý: `key_module_custom` (`materials.rs:183`) **đã** lọc insights theo
`domain.code_paths` rồi. Chỉ còn `dep_block` là không lọc.

**Sửa.** Thêm phép chiếu dep cho spec có fan-out — instance chỉ nhận slice
liên quan tới target của nó:

```rust
/// Chiếu output của dep xuống phần liên quan tới một fan-out target.
/// Không có phép chiếu cho (spec, dep) → trả nguyên bản (hành vi cũ).
fn project_dep(spec: &AgentSpec, dep: &str, target: Option<&FanTarget>, v: &Value) -> Value
```

- `key_module@D` / `deep_dive@D` ← `domain_modules`: chỉ entry của `D`,
  **cộng danh sách tên** của các domain còn lại (ngắn, ổn định) để giữ
  context "hệ thống có những module nào".
- `system_context` giữ nguyên với mọi instance — nó ổn định và là context
  chung thật sự.

**Vì sao làm trước manifest.** Không thêm state, không thêm path dependence,
không cần migration. Cache content-hash **hiện tại** lập tức cho
incremental per-domain. Rủi ro correctness thấp và review được.

**Đánh đổi.** `key_module` mất khả năng nhận xét so sánh giữa các domain
("module này trùng vai trò với module kia"). Giảm thiểu bằng danh sách tên
domain ở trên. Đây là đánh đổi chất lượng prose, không phải correctness.

**Đo.** Sửa 1 file trong 1 domain → `key_module` + `deep_dive` phải từ 14
call xuống 2. Bump `SCHEMA_VERSION` (đổi prompt shape → invalidate cache
của user hiện tại; nêu rõ trong commit message, theo CONTRIBUTING).

**Sau mảnh 0 phải đo lại trước khi làm tiếp** — rất có thể phần còn lại của
plan thu nhỏ đáng kể.

### Mảnh 1 — Manifest

`<internal>/manifest.json` (**không** nhét vào `state.json`: file đó giữ
quota theo ngày, `quota.rs:55` `DayState`, vòng đời reset hằng ngày).

- Lưu per-file + per-dir content hash. Diff ở đầu pipeline.
- **Chỉ chứa fingerprint, không chứa kết quả.** Kết quả vẫn ở content cache.
  Manifest thiếu / hỏng / sai version → fallback full run. Manifest không bao
  giờ là source of truth của nội dung.
- **Sửa luôn bug agentic (vấn đề 4):** đưa fingerprint vào cache key kể cả
  khi nó không nằm trong prompt →
  `Cache::key(prompt, model, backend, inputs)`.
- **Manifest key ⊇ cache key.** Đường tắt "cosmetic → skip" bỏ qua cache, nên
  manifest phải tự kiểm những thứ cache đang kiểm hộ:
  - `target_language` + `output_path` — repo này generate cả `docs/en` và
    `docs/vi`; đổi ngôn ngữ mà code không đổi **không được** ra "cosmetic".
  - hash của prompt set (`prompts/`, override được qua `prompts_dir`) +
    `SCHEMA_VERSION` + version binary.
  - config ảnh hưởng nội dung: `models.efficient/powerful`,
    `scan.excluded_dirs`, `limits.*`.

### Mảnh 2 — `agentwiki status`

Báo độ tươi: dirs đã đổi, commits kể từ lần research cuối, tuổi từng doc.
Có thể gộp vào `doctor`. Cho user quyết định có chủ đích thay vì chạy mò.

Repo có thể không phải git repo (`scan.git_tracked_only` là option) →
degrade gracefully: không có git thì chỉ báo theo dir hash + timestamp.

### Mảnh 3 — Significance classifier

Deterministic: dir count delta + import-graph edge delta (tái dùng
`drift::imports` / `drift::graph`). **Fail-open về structural.**

Nhánh **cosmetic là 0 call**, không phải "5-10 call": nếu reuse nguyên
`research.json` thì compose prompt không đổi → cache hit toàn bộ → docs
không đổi. Đây là kết quả tốt hơn và đáng quảng cáo — `agentwiki` thành
near-instant no-op khi chưa có gì đáng regenerate.

Hệ quả phải nói thẳng: **thay đổi cosmetic không bao giờ vào docs** cho tới
lần structural kế tiếp. Sửa typo thì ổn; sửa logic trong thân hàm mà không
đổi import thì docs mô tả hành vi cũ. `status` (mảnh 2) là chỗ bù lại — nó
phải hiện "N dir đổi cosmetic chưa phản ánh vào docs".

### Mảnh 5 — Selective compose

Deep-dive docs recompose theo domain delta; global docs recompose khi
research delta chạm tới. Mapping doc→input là **tĩnh, suy ra từ `registry`**
— không phải scheduler tổng quát. Fail-open về phía recompose khi không chắc.

### Mảnh 4 — Delta-research prompts *(gate, có thể không làm)*

Global agent nhận `{previous_output, delta}` thay vì full dossier, vẫn trả
JSON đầy đủ qua schema validation hiện có.

**Hạ xuống cuối cùng, chỉ làm sau khi mảnh 0/1/2/3 đã đo được lợi ích thật.**
Lý do: nó phá vỡ tính tái lập.

Output trở nên **phụ thuộc lịch sử, không phụ thuộc trạng thái repo**. Hai
repo giống hệt nhau — một clone mới chạy full, một update dần qua 10 delta —
cho ra docs khác nhau. Điều này làm mất tính chất quý nhất của cache hiện
tại: nó là *pure function của nội dung*, key sai thì chỉ miss chứ không bao
giờ trả kết quả bẩn, và `--force-regenerate` luôn về được trạng thái chuẩn.

Kéo theo:
- Cache gần như vô dụng cho các node này (prompt chứa `previous_output` →
  đổi mỗi run).
- Bug report không repro được: "docs sai" phụ thuộc chuỗi delta người dùng
  đã chạy, không phải commit hiện tại.
- Claim "prompt nhỏ = rẻ hơn" đáng nghi: `previous_output` của
  `domain_modules` / `system_context` không nhỏ, và `limits.materials_char_cap`
  đang là **192_000** ký tự — trần rất rộng, nên materials chưa chắc là thứ
  đang đắt. Phải đo trước.

Nếu vẫn làm: mỗi N lần incremental phải force một full run làm mốc tái lập,
và ghi `derived_from: <full-run-hash>` vào manifest.

## Chiến lược test

Ràng buộc "test offline bằng MockBackend" là đúng nhưng chưa đủ:
`tests/pipeline_offline.rs` hiện là **single-run**, trong khi correctness của
incremental chỉ chứng minh được bằng test **two-run**:

```
run 1 (full) → ghi lại tập call → mutate fixture → run 2 (incremental)
→ assert: đúng tập node dự kiến re-run
→ assert: output == full run trên fixture đã mutate   (invariant vàng)
```

- MockBackend cần record tập prompt/agent đã gọi, so sánh được giữa hai run.
- `tests/fixture-rs/` đã có import graph đủ phong phú (inline path, brace
  group, `super::`, `cfg(test)`, facade re-export) để dựng case structural vs
  cosmetic mà không cần fixture mới.
- Không sửa hình dạng `tests/fixture-app` — `pipeline_offline` phụ thuộc vào
  nó (ghi chú đã có từ plan drift).

## Đánh đổi — và mức độ nghiêm trọng

| Đánh đổi | Mức độ | Giảm thiểu |
|---|---|---|
| Delta-research bỏ sót hiệu ứng emergent (file nhỏ tạo cycle, đổi ý nghĩa kiến trúc toàn cục) | **Trung bình — risk chính** | Classifier dùng import-graph delta + `drift` verify sau run + periodic full-regen |
| Delta-research làm output phụ thuộc lịch sử, mất tái lập | **Trung bình** | Gate sau khi đo; full-run định kỳ làm mốc; `derived_from` trong manifest. Có thể bỏ hẳn mảnh 4 |
| Thay đổi cosmetic không vào docs tới lần structural kế | Trung bình | `status` hiện số dir cosmetic đang tồn đọng; user chủ động `--full` |
| Compose selective sai manifest → doc stale lặng lẽ | Trung bình | Fail-open về recompose; invariant two-run test; status báo tuổi từng doc |
| Manifest short-circuit bỏ sót input (ngôn ngữ, prompt, config) | Trung bình | Nguyên tắc **manifest key ⊇ cache key**; test đổi `--target-language` phải không ra cosmetic |
| Mảnh 0 làm `key_module` mất context liên-domain | Nhỏ | Vẫn truyền danh sách tên domain; là đánh đổi prose, không phải correctness |
| Prompt delta khó đúng (LLM lười copy / drop trường) | Nhỏ | Yêu cầu trả JSON đầy đủ + schema validation sẵn có |
| Manifest/state phức tạp hơn, cần migration | Nhỏ | File riêng, versioned, hỏng → full run |
| Classifier phân sai hướng nguy hiểm (structural→cosmetic) | Nhỏ nếu fail-open | Cosmetic→structural chỉ phí calls; chiều ngược lại mới độc — bias sang structural |
| Cache brittle với rename/move file | Vốn có, không đổi | Content-hash trung thực nhưng không "semantic" — chấp nhận |

**Đánh giá tổng**: đánh đổi không lớn nếu giữ đúng nguyên tắc —
*patch data, không patch prose; fail-open về phía regen; manifest key ⊇ cache
key; drift verify sau mỗi incremental run; full-regen định kỳ làm reset*.
Vấn đề đáng ngại nhất là prose decay — thiết kế này tránh nó bằng cách không
bao giờ patch prose. Vấn đề đáng ngại thứ hai là mất tái lập, và nó được cô
lập trọn trong mảnh 4 (mảnh duy nhất có thể bỏ).

## Ràng buộc

- Giữ `cargo fmt --check` + `cargo clippy --all-targets -- -D warnings` sạch.
- Test offline (MockBackend); không cần CLI thật cho unit/integration.
- Full-regen path hiện tại phải giữ nguyên semantics — incremental là
  opt-in (`--incremental` hoặc config), mặc định vẫn full.
- `drift` chạy sau incremental run như verify tầng cuối.
- Mảnh 0 và mảnh 1 đều đổi cache key → bump `SCHEMA_VERSION`, nêu trong
  commit message rằng cache của user bị invalidate.

## Thứ tự đề xuất

```
0. dep projection cho PerDomain fan-out   ← rẻ nhất, lợi ích lớn nhất, không thêm state
   ↓ ĐO LẠI ở đây trước khi đi tiếp
1. manifest (gồm fix agentic staleness + manifest key ⊇ cache key)
2. status
3. classifier (fail-open structural)
5. selective compose
4. delta-research   ← gate; có thể không cần làm
```

Mảnh 0 và 1 độc lập, ship riêng được. 1+2 đã đem lại visibility.

## Kết quả implement (2026-09-21)

| Mảnh | Trạng thái | Ghi chú |
|---|---|---|
| 0. Dep projection PerDomain | ✅ | `project_dep`/`project_domain_modules` trong `runner.rs`; `key_module`/`deep_dive` chỉ thấy slice domain mình + tên các domain khác |
| 1. Manifest + agentic fix | ✅ | `src/manifest.rs` (files/dirs/edges/env/git_head); `SCHEMA_VERSION` 2→3; cache key thêm `inputs` — `dir_summary` lấy subtree fingerprint, agentic-global lấy repo fingerprint |
| 2. `agentwiki status` | ✅ | `src/status.rs`; human + `--json`, exit 0, no-baseline → full run |
| 3. Classifier | ✅ | `Cosmetic`/`Structural(reasons)`, fail-open; cosmetic ⇒ **0-call no-op** khi artifacts còn nguyên |
| 5. Selective compose | ⚠️ rút gọn | Không cần bảng doc→input tĩnh: structural vẫn chạy full pipeline nhưng projection + cache làm call-level selective; cosmetic ⇒ no-op (đúng nhận định mảnh 3). Đã thêm cleanup deep-dive stale khi domain biến mất (`writer.rs`) |
| 4. Delta-research | ⏸ deferred | Giữ gate; đo lợi ích 0/1/2/3 trước |
| `--incremental` / `--full` / `incremental = true` | ✅ | Opt-in; mặc định full giữ nguyên |
| Drift verify sau incremental run | ✅ | `drift_verify_notice` warn-only trong `run_pipeline` |
| Tests | ✅ | `tests/incremental_offline.rs` 6 test (two-run, invariant incremental≡full, agentic invalidate, env change, status); manifest unit tests; `project_dep` + prompt-stability unit tests |

**Cache invalidation**: `SCHEMA_VERSION` 2→3 — cache cũ của user bị bỏ qua
(miss sạch, không lỗi), đúng như ràng buộc đã nêu.

**Lệch so với plan — chủ động**: mảnh 5 không build bảng doc→input tĩnh.
Lý do: sau mảnh 0, prompt của `deep_dive@<domain>` chỉ phụ thuộc slice
domain đó → cache hit tự nhiên khi domain không đổi → "selective" đạt được
ở mức call mà không cần scheduler/ma trận phụ thuộc mới. Compose phase vẫn
chạy lại (rẻ — đa số hit cache) nên docs luôn được viết lại từ data mới
nhất, không patch prose.

### Sửa sau review (F1–F7)

| # | Vấn đề | Sửa |
|---|---|---|
| F1 | `manifest.save()` trước compose/write → interrupt giữa chừng để manifest "claim" tree mà docs chưa viết | Save sau `write_docs` thành công, chỉ khi research thật sự chạy (`--skip-research` không advance fingerprint) |
| F2 | `docs_reusable` chỉ kiểm "research.json tồn tại + output là dir" → doc bị xoá vẫn báo "fresh" | Ground truth mới: `written-docs-<key>.json` (ghi cuối `write_docs`). No-op chỉ khi mọi doc đã ghi còn đúng trên đĩa **và** `research.json` không mới hơn record (research-save-không-compose → rerun) |
| F3 | `canonicalize` fallback giữ `./` → manifest slot đổi khi output dir mới tạo (orphan + `env_changed` giả) | `stable_abs`: normalize `.`/`..` lexical + canonicalize ancestor gần nhất — path ổn định trước/sau khi dir tồn tại |
| F4 | Test invariant `canned` map theo agent, bỏ qua prompt — "incremental ≡ full" overclaim | `hashing_backend` inject `sha256(prompt)` vào response → invariant phát biểu lại đúng: **structural rerun ≡ fresh full run** (cosmetic skip *cố ý* không regen docs — hợp đồng khác, đã có test riêng) |
| F5 | Cleanup xoá mọi `.md` lạ trong `4.Deep-Exploration/` — kể cả file user tự thêm | Deletion giới hạn bởi written-docs record của run trước — chỉ xoá file agentwiki đã ghi mà run này không còn |
| F6 | Mảnh 0 không có tác dụng ở agentic mode (global agents dùng repo fingerprint — đổi file nào cũng bust) | Đúng hướng an toàn (cwd repo-root đọc được mọi thứ); đã ghi vào README |
| F7 | `Manifest::build` vô điều kiện mỗi run — chi phí mới cho cả user không dùng incremental | `manifest: Option` — chỉ build khi `incremental || mode == Agentic`; `status` tự build on-demand |

**Phát hiện phụ từ F4**: prompt embed tên+path của project root (qua
root dossier) — hai bản copy ở path khác nhau không bao giờ cho doc tree
giống nhau byte-đối-byte. Invariant test vì vậy dùng *cùng một* project
dir với internal/output dir riêng biệt.

**Đánh đổi chấp nhận**: manifest chỉ được ghi bởi run `--incremental` hoặc
agentic — user embedded thuần không có baseline `status` (report in
"no-baseline" + hint). Trade chi phí build manifest mỗi run cho visibility
— chọn theo hướng không-tax cho user mặc định.

## Không làm — và lý do

- **Patch prose trực tiếp** — decay tích lũy vô hình (xem góc nhìn
  Data vs Prose).
- **Incremental DAG scheduler tổng quát** (input-manifest per agent,
  dependency-aware scheduling động) — cache content-hash đã làm việc tương
  đương ở mức call. Mảnh 5 có mapping doc→input nhưng là **bảng tĩnh suy ra
  từ `registry`**, không phải scheduler.
- **Regen mỗi commit trong CI** — `drift --strict` đã cover khoảng giữa
  hai lần regen; docs không cần tươi tới từng commit.
- **Semantic/embedding-based change detection** — overkill; import-graph
  delta là proxy đủ tốt và deterministic.
