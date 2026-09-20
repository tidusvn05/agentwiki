# Kế hoạch: `agentwiki drift` — drift check ít nhiễu giữa generated edges và import edges

## Context

Câu hỏi gốc: *"Opt-in drift check nghe hợp lý, cho tới khi generated edges và import edges lệch nhau vì lý do vớ vẩn. Lọc false positive thế nào trước khi CI noise thắng?"*

Hiện agentwiki chỉ có LLM sinh edge (`relationships.core_dependencies` trong `.agentwiki/research.json`), không có import graph tĩnh, không có resolver import → file, và `verify` không kiểm tra tính đúng của edge. Kế hoạch này thêm subcommand read-only, không gọi LLM, so sánh hai tập edge với một chuỗi bộ lọc false positive có tên, test được từng cái.

Quyết định đã chốt với user:
1. Claimed edges = `relationships.core_dependencies` (có cấu trúc). `domain_relations` và Mermaid trong docs để v2.
2. Chạy bằng subcommand `agentwiki drift` kiểu `doctor`, không gắn vào pipeline.
3. Mặc định exit 0 (chỉ cảnh báo). `--strict` chỉ fail khi có phantom/reversed **mới** so với baseline. Edge thiếu trong doc không bao giờ fail.

Phát hiện khi khảo sát (ảnh hưởng thiết kế):
- `.agentwiki/` bị gitignore → CI không có `research.json`. Cần file claims được commit.
- Endpoint có thể là **file** (`src/main.py -> src/api.py` ở fixture) hoặc **thư mục** (`src/pipeline -> src/agent` ở repo này), đôi khi có hậu tố tự do (`src (config.rs, cli.rs…)`).
- Repo này có edge thật chỉ tồn tại dưới dạng inline path, không có `use`: `src/pipeline/mod.rs:216,224,225` gọi `crate::output::…`. Extractor chỉ bắt `use` sẽ báo phantom sai.
- Doc comment chứa `crate::…` (`src/cli.rs:1`) → phải xóa comment/string trước khi match.
- `tests/*.rs` và `src/main.rs` import qua tên crate `agentwiki::…` → resolver phải map `[package].name` về crate root.
- `scanner::insights::extract` không dùng lại được: regex Rust không xử lý `use crate::{a, b}`, cap `.take(40)`, và output của nó đi vào prompt `dir_summary` — sửa nó sẽ làm mất cache của mọi user. Drift cần extractor riêng.
- `DependencyType::Module` là giá trị default/catch-all (5/13 edge thật) → vẫn kiểm tra được nhưng độ tin cậy thấp hơn `Import`.

## Nguyên tắc chủ đạo: bằng chứng bất đối xứng

- Kiểm tra **phantom/reversed** dùng đồ thị rộng nhất `G_full` (gồm import trong test, inline path, edge yếu đi qua re-export). Càng nhiều bằng chứng càng ít phantom sai.
- Kiểm tra **undocumented** dùng đồ thị chặt nhất `G_strict` (không test, chỉ bằng chứng mạnh, bỏ hub và file tiện ích, có ngưỡng số lượng).
- Khai báo `mod x;` tách riêng: chỉ xác nhận cặp cha–con, không dùng cho đường bắc cầu hay undocumented (nếu không `lib.rs` làm mọi node reachable, không bao giờ có phantom).

## Bố cục module (mỗi file < ~400 dòng, dùng `BTreeMap/BTreeSet` để output ổn định)

```
src/diag.rs                   Status / Report / print_section chuyển từ doctor.rs (pub(crate))
src/drift/mod.rs              run(), analyze(), DriftOptions, exit code
src/drift/config.rs           DriftConfig + DriftPartial + apply()
src/drift/claims.rs           load_claims(), export_claims(), chuẩn hóa endpoint
src/drift/imports/mod.rs      Lang, RawImport, ImportSpec, EvidenceKind, extract_imports()
src/drift/imports/sanitize.rs xóa comment + string, giữ offset
src/drift/imports/{rust,python,js}.rs
src/drift/resolve/mod.rs      RepoIndex, Resolution, resolve()
src/drift/resolve/{rust,python,js}.rs
src/drift/graph.rs            FileGraph, NodeSet, NodeGraph, lift(), reachable(), hubs(), re-export closure
src/drift/compare.rs          chuỗi lọc cho claimed edge (C1–C12)
src/drift/undocumented.rs     chuỗi lọc cho undocumented edge (U1–U9)
src/drift/findings.rs         FindingClass, Reason, Finding, finding_id()
src/drift/baseline.rs
src/drift/report.rs           DriftReport (Serialize) + renderer
```

Chữ ký chính:
- `pub fn run(project_path, config_path, opts: DriftOptions) -> i32` — nơi duy nhất `println!`.
- `pub fn analyze(cfg: &DriftConfig, scan: &ScanData, claims: &[CoreDependency], baseline: Option<&Baseline>) -> DriftReport` — thuần, mục tiêu test chính.
- `pub fn load_claims(path) -> Result<Vec<CoreDependency>>` — serde đồng bộ (không dùng `ResearchContext::get_typed` vì nó nuốt lỗi parse); nhận cả map research đầy đủ lẫn `{relationships}` rút gọn.
- `resolve(index, importer, imp) -> Resolution::{Internal(rel_file) | External | Unresolved}`.
- `NodeGraph::reachable(a, b, max_depth, blocked) -> Option<Vec<NodeId>>` — BFS, chọn đường nhỏ nhất theo thứ tự từ điển.

Tái sử dụng: `scanner::scan(&Config)` + `ScanData/FileEntry/DirectoryInfo` (cùng tập file LLM đã thấy), `CoreDependency`/`DependencyType::as_str` (`src/agent/reports/relationship.rs`), `util::write_atomic`, `crate::error::{Error, Result}`, mẫu config 3 phần của `VerifyConfig` (`src/config.rs:194-208, 290, 553-557`), mẫu dispatch của `doctor` (`src/main.rs:16-24`). Extractor đọc file bằng `std::fs` (không dùng `read_capped`). Không thêm dependency mới (regex/glob/toml/serde_json đã có).

## Trích xuất & resolve import

- **Rust**: sanitize → tìm mọi `use …;` (kể cả nhiều dòng) → `expand_use_tree` (brace lồng nhau, `as`, `self`, `*`) → xóa các `use` rồi match `\b(crate|super|self|<tên_crate>)(::ident)+` để bắt inline path. Theo dõi `mod x { }` theo độ sâu ngoặc để resolve `super`; `#[cfg(test)]` → `test_only`. `pub use` → `ReExport`. Index dựng từ vị trí file: `Cargo.toml` gần nhất (`package.name`, `-`→`_`), root là `src/lib.rs` hoặc `src/main.rs`; mỗi file trong `tests/ examples/ benches/ src/bin/` là một crate root. Resolve theo prefix module dài nhất.
- **Python**: `from .x import y`, `from . import x`, `from ..p import z`, import nhiều dòng trong ngoặc; thử tên import là submodule trước (`p/z.py`, `p/z/__init__.py`); import tuyệt đối qua index hậu tố đường dẫn, mơ hồ → `Unresolved`. `from`-import trong `__init__.py` → `ReExport`.
- **JS/TS**: `import…from`, `import 'x'`, `export…from` (ReExport), `require()`, `import()`. Specifier tương đối: thử đúng path → các đuôi `.ts .tsx .d.ts .js .jsx .mjs .cjs .json .vue .svelte` → `/index.*`; map `./x.js`→`./x.ts`. Alias `@/`, `~/` → `Unresolved` (tsconfig paths để sau).
- **Ngôn ngữ khác**: `Lang::UnsupportedCode`, không trích edge; guard coverage biến claim thành `unverifiable` thay vì phantom.
- **Đi qua facade** (`graph.rs`): với edge `X→F` mà `F` là `lib.rs/mod.rs/__init__.py/index.*`, thêm edge yếu `X→T` theo closure `ReExport` của `F` (sâu ≤ 3), chỉ vào `G_full`.

## Node & chuỗi bộ lọc

**Chuẩn hóa endpoint**: trim, bỏ backtick, `./` đầu, `/` cuối, `\`→`/`, bỏ hậu tố trong ngoặc; khớp file chính xác → thư mục chính xác → hậu tố đường dẫn duy nhất; không khớp → `Unknown`. Không fuzzy theo tên.

**Nâng file → node**: file là chính nó nếu là file-node; nếu không thì thuộc thư mục claimed tổ tiên sâu nhất; nếu không thì node ẩn của thư mục cha (tham gia reachability, không xuất hiện trong finding). Claim trùng `(from,to)` được gộp.

**Claimed edge — verdict đầu tiên thắng** (kiểm tra dương trước, guard sau, âm cuối cùng):

| # | Bộ lọc | Kết quả |
|---|---|---|
| C1 | `empty_endpoint` | unverifiable |
| C2 | `unknown_endpoint` | unverifiable |
| C3 | `self_loop` | structural |
| C4 | `containment` (đầu này là tổ tiên đầu kia) | confirmed nếu có bằng chứng (kể cả `mod`), ngược lại structural. Không bao giờ phantom |
| C5 | `direct_evidence` trên `G_full` | confirmed (direct) |
| C6 | `transitive` ≤ `max_transitive_depth`, chặn hub làm trung gian | confirmed (kèm đường đi) |
| C7 | `kind_not_checkable` (DataFlow hoặc ngoài `checkable_kinds`) | unverifiable |
| C8 | `non_code_endpoint` (không có file code, hoặc thư mục có trên đĩa nhưng 0 file được quét) | unverifiable |
| C9 | `language_coverage` < `min_language_coverage` ở một đầu | unverifiable |
| C10 | `resolver_confidence` (tỉ lệ unresolved của node nguồn > `max_unresolved_ratio`, hoặc repo có 0 edge nội bộ) | unverifiable |
| C11 | `reversed` (có edge trực tiếp B→A) | **reversed** |
| C12 | còn lại | **phantom** |

**Undocumented edge** (trên `G_strict`, đếm số lượng bị chặn ở mỗi bộ lọc): U1 cả hai đầu là claimed node · U2 bỏ cặp cha–con · U3 bỏ bằng chứng test-only và `mod` · U4 `ignore_nodes` / `ignore_files` · U5 bỏ edge trỏ vào hub (in-degree ≥ `hub_in_degree_ratio`, chỉ bật khi ≥ `hub_min_nodes` node) · U6 bỏ nếu đã có claim B→A bất kỳ kind nào · U7 bỏ nếu claims đã có đường A→…→B · U8 cần ≥ `min_undocumented_imports` cặp (importer, target) · U9 cắt ở `max_undocumented_reported`.

**Phân loại**: `phantom`, `reversed` (warn; fail dưới `--strict` nếu mới) · `undocumented` (warn) · `unverifiable`, `structural` (info) · `confirmed` (ok). ID = `<class>:<from_node>-><to_node>` theo path đã chuẩn hóa, **không chứa kind** để LLM đổi nhãn không tạo finding mới.

## Claims, baseline, config, CLI

- **Nguồn claims** theo thứ tự: `--claims` → `[drift] claims_path` → `<internal>/research.json` → `<project>/.agentwiki-drift-claims.json`. Header báo cáo luôn in nguồn đang dùng.
- **`--export-claims [PATH]`**: ghi `{relationships}` rút gọn từ `research.json` ra file commit được (mặc định `.agentwiki-drift-claims.json`) — giải quyết việc `.agentwiki/` bị gitignore, không cần `jq`.
- **Baseline**: `<project>/.agentwiki-drift-baseline.json` (tên có dấu chấm đầu → không bị scanner/prompt nuốt khi `include_hidden=false`); `{schema_version, findings:[ids]}` đã sort, gồm phantom/reversed/undocumented. `--update-baseline` ghi lại và exit 0 (`conflicts_with = "strict"`). Entry cũ không còn khớp → info.
- **`[drift]`** (struct ở `src/drift/config.rs`; `src/config.rs` chỉ thêm field + `TomlConfig.drift` + 1 arm): `claims_path=None`, `baseline_path`, `max_transitive_depth=3`, `checkable_kinds=[import,function_call,inheritance,composition,module]`, `min_language_coverage=0.8`, `max_unresolved_ratio=0.25`, `hub_in_degree_ratio=0.5`, `hub_min_nodes=5`, `min_undocumented_imports=3`, `max_undocumented_reported=20`, `ignore_nodes=[]`, `ignore_files=[**/error.*, **/errors.*, **/config.*, **/util(s).*, **/utils/**, **/types.*, **/constants.*, **/prelude.*]`, `test_globs=[tests/**, test/**, **/__tests__/**, **/*_test.*, **/test_*.py, **/*.spec.*, **/*.test.*, **/conftest.py, benches/**, examples/**]`, `exclude_cfg_test=true`.
- **`DriftArgs`** (`src/cli.rs`, cạnh `Command::Doctor`): `-p`, `-c`, `--strict`, `--update-baseline`, `--export-claims`, `--json`, `--max-depth`, `--claims`, `--baseline`. Cập nhật comment `cli.rs:20` (`drift` giờ che profile cùng tên).
- **Dispatch** trong `src/main.rs` trước `Config::load` chính như `doctor` (không run lock, không signal handler, không backend). `drift::run` tự gọi `Config::load`; lỗi config là fatal. Với `--json`, tracing ghi ra stderr.
- **Exit code**: 0 sạch/chỉ cảnh báo · 1 khi `--strict` và có ≥1 phantom/reversed mới · 2 khi không chạy được (lỗi config, thiếu/hỏng claims, scan lỗi). Thiếu claims luôn là 2 kèm thông báo nêu path và cách khắc phục.
- **Output**: báo cáo người đọc (nguồn claims, đếm tổng, phantom/reversed kèm kind + importance + tối đa 3 ví dụ `file:line`, undocumented kèm số lượng, info gộp theo lý do trừ khi `-v`, footer "N finding sẽ fail dưới --strict") + `<internal>/drift.json` qua `write_atomic` (ghi lỗi chỉ `tracing::warn!`).

## Các bước (mỗi bước merge độc lập, không bước nào tự sinh phantom sai)

- **M0** Tách `src/diag.rs` từ `doctor.rs` (không đổi hành vi).
- **M1** CLI + config + `claims.rs` + node + exit code; mọi edge ra structural/unverifiable.
- **M2a** `sanitize` + extractor Rust · **M2b** `RepoIndex` + resolver Rust · **M2c** graph, lift, facade closure, C1–C12.
- **M3** Python · **M4** JS/TS.
- **M5** baseline + `--strict` + `--export-claims`.
- **M6** U1–U9 + hub.
- **M7** `drift.json`, `--json`, README (`### agentwiki drift` sau `doctor`, khối `[drift]`, snippet CI 3 giai đoạn), CONTRIBUTING ("ngôn ngữ mới = 1 extractor + 1 resolver"), `tests/fixture-rs/`, self-test.
- **Sau này**: Go/Java, tsconfig paths, `domain_relations`, Mermaid trong docs, pipeline tự xuất claims.

Rollout CI ghi trong README:
```yaml
# 1. Quan sát
- run: agentwiki drift
  continue-on-error: true
# 2. Chạy `agentwiki drift --update-baseline` ở local, commit baseline + claims
# 3. Gate
- run: agentwiki drift --strict
```

## Verification

- **Unit test trong module**: `sanitize`, `expand_use_tree`, từng extractor/resolver (layout dựng bằng `assert_fs`), từng bộ lọc C/U trên `NodeGraph` dựng tay, diff baseline, độ ổn định finding ID, test merge `[drift]` trong `src/config.rs` tests.
- **`tests/drift_offline.rs` trên `tests/fixture-app`** (claims viết inline bằng `serde_json::json!`, `scan.git_tracked_only=false`, không thêm file vào fixture vì `pipeline_offline` phụ thuộc hình dạng của nó). Kỳ vọng: 4 edge confirmed direct; `storage.py -> db/schema.sql` unverifiable; `main.py -> models.py` confirmed bắc cầu qua `api.py`; claim giả `models.py -> api.py` → reversed; `models.py -> main.py` → phantom; `main.py -> storage.py` chỉ hiện undocumented khi `min_undocumented_imports=1`. Kiểm tra exit code 0 / 1 (`--strict`) / 0 sau `--update-baseline` / 2 khi thiếu claims.
- **`tests/fixture-rs/`** (nhỏ, không bao giờ compile): edge chỉ có inline path, brace group, `super::`, import trong `cfg(test)`, mồi trong doc comment, facade re-export, `tests/it.rs` dùng tên crate, 1 phantom thật, 1 reversed thật.
- **Self-check trên repo này** (`cargo run -- drift -v`): 13 edge thật phải cho **0 phantom, 0 reversed** — 3 confirmed qua containment (`src -> src/pipeline`, `src/agent -> src/agent/reports`, `src/backend -> src`), 5 confirmed direct (gồm `src/pipeline -> src/output` chỉ nhờ inline path và `tests -> src/backend` nhờ tên crate), 4 unverifiable do kind DataFlow, 1 unverifiable non-code (`prompts/editors -> prompts`); `src` được nhận là hub.
- **CI gate của repo**: `cargo clippy --all-targets -- -D warnings`, `cargo test --all-targets`, `cargo build --release --locked`; `cargo fmt` (max_width 100). Không `unwrap/expect` ngoài test, `//!` cho mỗi file, `///` cho item public. Không đổi `cache::SCHEMA_VERSION` (không đụng prompt/cache).

## Rủi ro

- Regex bỏ sót path sinh từ macro, `#[path]`, glob re-export → giảm nhẹ bằng `G_full` rộng + guard C10.
- Nới lỏng bắc cầu/facade làm giảm recall của phantom — chấp nhận, đúng với chính sách "chỉ fail tín hiệu mạnh".
- Sinh lại docs có thể đổi tên endpoint → ID đã baseline hiện lại như mới; hợp lý vì PR sinh lại docs nên review chúng.
- 13 tham số config là nhiều → README chỉ nêu nổi bật `claims_path`, `baseline_path`, `ignore_nodes`, `max_transitive_depth`.
