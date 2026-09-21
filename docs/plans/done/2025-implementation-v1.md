# AgentWiki — Implementation Plan

> **Status: hoàn thành (M0–M6), không maintain.** Viết trước commit đầu tiên
> (`dba8c16`); ship xong ở v0.5.0. Giữ lại vì §2 (port gì / bỏ gì từ
> deepwiki-rs) và §14 (rủi ro) là context không suy ra được từ code.
>
> **Không dùng file này làm mô tả kiến trúc hiện tại** — nó đã lệch đáng kể.
> Kiến trúc hiện tại: [`docs/en/2.Architecture.md`](../../en/2.Architecture.md)
> (generated, verify bằng `agentwiki drift`).
>
> Các điểm đã biết là lệch: §4 liệt kê `pipeline/{orchestrator,phases}.rs`,
> `output/doctree.rs`, `verify/*` (không tồn tại) và thiếu hẳn `src/drift/**`,
> `doctor.rs`, `diag.rs`, `progress.rs`, `sys.rs`, `agent/reports/*`;
> §5–§6 thiếu field thêm về sau (`AgentSpec.materials/phase/exec`,
> `AgentRequest.agent/json_schema`, `ExecKind::Deterministic`); §10 thiếu
> `[profiles.*]` và `[drift]`; §15 đã chốt hết; `agentwiki.toml.example` ở §4
> không tồn tại.
>
> **Defect thiết kế đã biết — đừng port lại:** §9 định nghĩa cache key là
> `sha256(prompt ‖ model ‖ backend ‖ SCHEMA_VERSION)`. Công thức này đúng với
> mode `embedded` (§8) nhưng **sai với `agentic`**: ở mode đó code không nằm
> trong prompt, nên sửa code không đổi key → cache hit → docs stale âm thầm.
> Plan định nghĩa hai mode ở §8 rồi định nghĩa cache ở §9 mà không nối lại.
> Đang xử lý trong [`../active/incremental-update.md`](../active/incremental-update.md).

> Rewrite of [deepwiki-rs (Litho)](https://github.com/sopaco/deepwiki-rs) as a
> CLI-agent-native documentation generator, **in Rust**. Instead of an
> OpenAI-compatible HTTP API, each pipeline agent is backed by an
> authenticated agent CLI (`devin`, `claude`, `codex`, …) through a
> trait-based backend interface.

## 1. Mục tiêu & phi mục tiêu

**Mục tiêu**
- Sinh bộ tài liệu kiến trúc kiểu C4 (Overview / Architecture / Workflow /
  Boundary / Deep-dive / Database) cho một repo bất kỳ.
- LLM backend = CLI đã auth sẵn trên máy (không API key trả phí).
- **v1: chỉ hỗ trợ `devin`**, nhưng backend là trait + factory → thêm
  `claude`/`codex` chỉ bằng 1 file mới + 1 dòng register.
- Kiến trúc module rõ ràng, mở rộng được: backend mới, agent mới, output
  format mới đều là điểm cắm plugin.
- Cache kết quả theo content-hash — CLI call đắt (10–160s), không gọi lại
  khi không cần.

**Phi mục tiêu (v1)**
- Không làm HTTP API, MCP server, hay web UI.
- Không port ReAct tool-calling của Litho (thay bằng "agentic mode", mục 8).
- Không hỗ trợ multi-provider HTTP (OpenAI/DeepSeek/…) — cần thì dùng bản
  gốc deepwiki-rs.

## 2. Bài học từ deepwiki-rs (port gì, bỏ gì)

| Giữ lại (concept) | Bỏ / thay thế |
|---|---|
| Pipeline 4 phase: Preprocess → Research → Compose → Verify | rig-core + toàn bộ LLM provider matrix → `AgentBackend` trait |
| Agent task = prompt + JSON schema + retry-with-error-feedback | ReAct preset tools → agentic mode của CLI |
| Cache `.litho/cache` theo hash | `--disable-preset-tools` hack (không còn cần) |
| `litho.toml` config | |
| Output tree `1.Overview … 6.Database-Overview` | |
| Mermaid verify pass cuối (gọi `mermaid-fixer` binary nếu có) | |

Điểm khác biệt cốt lõi: deepwiki-rs phải **nhét code vào prompt** vì model
API không đọc được filesystem. Agent CLI **tự đọc file được** → 2 chế độ
context (mục 8). v1 default là `embedded` (deterministic, dễ debug);
`agentic` là opt-in.

## 3. Công nghệ & dependencies

- **Rust stable 1.98** (đã có trên VPS), edition 2024.
- Một package duy nhất, layout **lib + bin**: `src/lib.rs` expose toàn bộ
  core, `src/main.rs` chỉ là thin CLI wrapper → crate khác/test/integration
  reuse được, sau này tách workspace (`agentwiki-core`, `agentwiki-backends`)
  cũng dễ.
- Async runtime: **tokio** (subprocess fan-out qua `tokio::process`,
  parallelism qua `JoinSet` + `Semaphore`).

| Crate | Dùng cho |
|---|---|
| `clap` (derive) | CLI parsing |
| `serde`, `serde_json`, `schemars` | config, structured output, JSON schema inject vào prompt |
| `toml` | `agentwiki.toml` |
| `tokio` (rt-multi-thread, process, macros, time, sync) | runtime + subprocess + semaphore |
| `async-trait` | object-safe `AgentBackend` |
| `thiserror` / `anyhow` | error ở lib / ở binary boundary |
| `tracing`, `tracing-subscriber` | logging chuẩn, có span per-agent |
| `walkdir`, `glob`, `regex` | scanner |
| `sha2`, `hex` | cache key |
| `time` | timestamp cho call log / cache meta |
| `tempfile`, `assert_fs` (dev) | tests |

Không dùng reqwest/HTTP stack — không cần.

## 4. Cấu trúc module

```
agentwiki/
├── IMPLEMENTATION_PLAN.md          # file này (nay: docs/plans/done/2025-implementation-v1.md)
├── Cargo.toml
├── rustfmt.toml
├── clippy.toml
├── prompts/                        # template, override-able, default embed bằng include_str!
│   ├── dir_summary.md
│   ├── system_context.md
│   ├── domain_modules.md
│   ├── architecture.md
│   ├── workflow.md
│   ├── key_module.md
│   ├── boundary.md
│   ├── database.md
│   └── editors/                  # overview.md, architecture_doc.md, …
├── src/
│   ├── main.rs                   # thin: parse args → run → exit code
│   ├── lib.rs                    # pub mod + re-export API surface
│   ├── cli.rs                    # clap Args → Config overrides
│   ├── config.rs                 # agentwiki.toml + merge precedence
│   ├── error.rs                  # Error enum (thiserror), Result<T>
│   ├── scanner/
│   │   ├── mod.rs
│   │   ├── files.rs              # walk + exclude rules + git-tracked filter
│   │   ├── structure.rs          # tree, per-dir dossier input
│   │   └── insights.rs           # heuristic code insights (fn/class/import…)
│   ├── agent/
│   │   ├── mod.rs
│   │   ├── spec.rs               # AgentSpec { name, prompt_tmpl, schema, tier, deps, fan_out }
│   │   ├── registry.rs           # toàn bộ task DAG (mục 6)
│   │   ├── runner.rs             # render → backend.run → parse → validate → retry
│   │   └── context.rs            # ResearchContext: kết quả agent trước, có kiểu
│   ├── backend/
│   │   ├── mod.rs                # trait AgentBackend + AgentResult + BackendKind + factory
│   │   ├── devin.rs              # v1
│   │   ├── claude.rs             # stub: Error::BackendNotAvailable("claude")
│   │   └── codex.rs              # stub
│   ├── pipeline/
│   │   ├── mod.rs
│   │   ├── orchestrator.rs       # resolve deps → JoinSet + Semaphore → ResearchContext
│   │   └── phases.rs             # Scan/Research/Compose/Verify wiring
│   ├── cache.rs                  # sha256(prompt+model+schema_ver) → .agentwiki/cache/
│   ├── quota.rs                  # daily cap + calls.jsonl audit
│   ├── output/
│   │   ├── mod.rs
│   │   ├── doctree.rs            # thứ tự/tên file output
│   │   └── writer.rs             # ghi markdown + summary report
│   └── verify/
│       ├── mod.rs
│       ├── mermaid.rs            # detect ```mermaid, gọi mermaid-fixer nếu which() thấy
│       └── integrity.rs          # coverage check, broken refs
├── tests/
│   ├── common/mock_backend.rs    # MockBackend: canned JSON, record prompts
│   ├── fixture-app/              # mini repo ~5 file (port từ /tmp/demo-app)
│   ├── pipeline_offline.rs       # full DAG với MockBackend
│   └── e2e_devin.rs              # #[ignore] trừ khi AGENTWIKI_E2E=1
└── agentwiki.toml.example
```

## 5. Backend abstraction (core)

```rust
// src/backend/mod.rs
#[derive(Debug)]
pub struct AgentResult {
    pub text: String,
    pub backend: BackendKind,
    pub model: Option<String>,
    pub duration: Duration,
    pub usage: Option<TokenUsage>,   // để None nếu CLI không expose
    pub stderr_tail: String,
}

#[async_trait]
pub trait AgentBackend: Send + Sync {
    fn kind(&self) -> BackendKind;
    /// Agent có thể tự đọc file trong `cwd` không (agentic mode).
    fn supports_fs(&self) -> bool;
    async fn run(&self, req: AgentRequest) -> Result<AgentResult>;
}

pub struct AgentRequest {
    pub prompt: String,
    pub model: Option<String>,
    pub cwd: PathBuf,      // empty-cwd (embedded) | project root (agentic)
    pub timeout: Duration,
}
```

- Model string quy ước `"<backend>:<model>"` — vd `devin:swe-2-medium`.
  `BackendKind::parse` tách phần backend; phần sau `:` truyền nguyên cho CLI.
- Config 2 tier: `model_efficient` (task thường), `model_powerful`
  (reasoning nặng + fallback khi efficient fail hết retry).
- Factory: `backend::for_kind(BackendKind) -> Arc<dyn AgentBackend>` —
  backend mới = 1 file + 1 arm trong match.

### DevinBackend (v1)

```
cmd: devin -p --prompt-file <tmp> --respect-workspace-trust false [--model m]
in:  prompt → tempfile (tránh giới hạn argv)
out: stdout = final message
env: Command::env_remove cho ANTHROPIC_*/OPENAI_*/DEVIN_API*/OPENHANDS_*
```

Edge đã biết (đã gặp khi test proxy): model sai → exit 1 kèm list models;
non-zero exit → `Error::Backend` kèm stderr tail 500 chars.

### Stub backends (v2)

- `ClaudeBackend`: `claude -p --output-format text --no-session-persistence
  --permission-mode bypassPermissions --setting-sources local [--model m]`,
  prompt qua stdin.
- `CodexBackend`: `codex exec --skip-git-repo-check -s read-only
  -o <tmp> [-m m] -`, prompt qua stdin, đọc last message từ `-o`.
- Cả hai đã verify hoạt động qua `~/workspace/cli-llm-proxy` → port lệnh gọi
  là copy-paste; mỗi backend 1 hàm `build_cmd()` duy nhất để dễ vá khi CLI
  đổi flag.

## 6. Pipeline & task graph

Phase 0 — **Scan (không AI)**:
`scan_files` → `build_structure` → `extract_docs` (README…) →
`extract_insights` → git-tracked filter. Tất cả deterministic, chạy sync
trong `tokio::task::spawn_blocking` nếu repo lớn.

Phase 1 — **Research**: mỗi node = 1 `AgentSpec`, orchestrator topo-sort theo
`deps`, node cùng level chạy song song qua `Semaphore(max_parallels)`:

```
structure ──┬─> dir_summary[per-dir]        (fan-out)
            ├─> relationships
            └─> system_context ─┐
                  domain_modules ─┬─> key_module[per-domain]  (fan-out)
                  architecture    │
                  workflow        ├─> boundary
                  database        │
```

Kết quả mỗi agent đi vào `ResearchContext` (typed store, thay "Memory" của
Litho) — agent sau đọc kết quả agent trong `deps` của nó.

Phase 2 — **Compose**: `overview`, `architecture_doc`, `workflow_doc`,
`boundary_doc`, `deep_dive[per-domain]`, `database_doc`. Editor nhận
research context liên quan, trả **markdown trần** (`schema = None`).

Phase 3 — **Verify**: detect ```mermaid blocks → gọi `mermaid-fixer` binary
nếu `which` thấy (không thì log skip), integrity check, ghi summary report.

```rust
pub struct AgentSpec {
    pub name: &'static str,
    pub prompt_tmpl: &'static str,       // tên file trong prompts/
    pub schema: Option<SchemaRef>,       // schemars schema → inject vào prompt
    pub tier: ModelTier,                 // Efficient | Powerful
    pub deps: &'static [&'static str],
    pub fan_out: Option<FanOut>,         // PerDir | PerDomain
}
```

## 7. Structured output & retry

Giữ cơ chế Litho (đã verify hoạt động):
1. Prompt = base + "**YOU MUST RETURN VALID JSON**" + schema pretty-printed
   (`schemars::schema_for!(T)`).
2. Parse chain: `serde_json::from_str` → tách ```json block → tách `{…}`
   đầu tiên (depth-counting) → strip fence rồi parse.
3. Validate: `serde_json::from_value::<T>` — schema là source of truth.
4. Fail → retry tối đa `retry_attempts` (default 3), prompt retry kèm
   "**Previous attempt failed: {err}**"; hết retry với tier Efficient →
   thử lại 1 lần bằng model Powerful (fallback hiện có của Litho).

## 8. Context modes

| | `embedded` (default) | `agentic` (`--agentic`) |
|---|---|---|
| Agent cwd | `<internal>/empty-cwd/` | project root |
| Code vào prompt | có, từ scanner | không nhét — prompt chỉ dẫn "đọc repo tại cwd" |
| Determinism | cao | thấp hơn |
| Token/call | cao | thấp hơn nhiều |
| Rủi ro | — | agent dùng tool linh tinh → prompt kèm "READ-ONLY, do not modify files" |

`supports_fs()` của backend quyết định agentic có khả dụng không.

## 9. Cache, parallelism, quota

- **Cache** (`cache.rs`): key = sha256(prompt ‖ model ‖ backend ‖
  SCHEMA_VERSION) → `.agentwiki/cache/<key>.json` { result, meta }.
  Flags `--no-cache`, `--force-regenerate`.
- **Parallelism**: `Semaphore::new(config.max_parallels)` quanh mọi
  `backend.run`; fan-out agents join qua `JoinSet`. Default 2.
- **Quota** (`quota.rs`, theo rule nội bộ): `.agentwiki/state.json` đếm CLI
  call/ngày, vượt `daily_cap` (default 300) → `Error::QuotaExceeded`
  fail-fast. Mọi call append 1 dòng JSONL vào `.agentwiki/calls.jsonl`
  { ts, agent, backend, model, prompt_chars, secs, status } để audit.
- **Retry/timeout**: `tokio::time::timeout(config.call_timeout)` quanh
  process; non-zero exit → error kèm stderr tail.

## 10. Config (`agentwiki.toml`)

```toml
project_path = "."
output_path = "./agentwiki.docs"
target_language = "en"               # zh en ja ko de fr ru vi
max_parallels = 2
mode = "embedded"                    # | agentic

[models]
efficient = "devin:swe-2-medium"
powerful  = "devin:swe-2-medium"

[limits]
daily_cap = 300
call_timeout_s = 600
retry_attempts = 3

[scan]
max_depth = 10
max_file_size = 524288
git_tracked_only = true
excluded_dirs = [".git", "node_modules", "target", "dist", "__pycache__"]
excluded_files = ["*.lock", "*.log", ".env"]

[verify]
mermaid_fixer = true                 # gọi binary nếu có, không thì skip
```

CLI overrides: `-p/-o/-c`, `--target-language`, `--model-efficient`,
`--model-powerful`, `--max-parallels`, `--skip-research`,
`--skip-documentation`, `--agentic`, `--no-cache`, `--force-regenerate`,
`-v`. Precedence: CLI > toml > default.

## 11. Coding standards

- **Format**: `cargo fmt` với `rustfmt.toml` (imports granularity, wrap
  comments); `cargo clippy --all-targets -- -D warnings` sạch trước merge.
- **Errors**: lib code dùng `thiserror` `Error` enum + `Result<T>`;
  `anyhow` chỉ ở `main.rs`. Không `unwrap`/`expect` trong lib (trừ test).
- **Docs**: `///` cho mọi public item; `//!` module doc đầu mỗi file.
- **Size**: file > ~400 dòng thì tách module; 1 trait-impl = 1 file.
- **Logging**: `tracing` spans `agent=<name> backend=<kind>`; không in
  prompt ra stdout (chỉ debug level, truncate).
- **Secrets**: không bao giờ ghi env var nhạy cảm vào log/file;
  `env_remove` list định nghĩa 1 chỗ duy nhất (`backend::sanitized_env`).
- **Tests**: unit test cạnh module; `pipeline_offline.rs` chạy full DAG bằng
  `MockBackend` — CI/test thường không cần CLI auth; `e2e_devin.rs`
  `#[ignore]` mặc định, chạy khi `AGENTWIKI_E2E=1`.
- **Naming/commit**: snake_case module, CamelCase type; commit nhỏ theo
  milestone.

## 12. Milestones & acceptance

| # | Deliverable | Accept khi |
|---|---|---|
| M0 | Cargo skeleton: lib+bin, cli.rs, config.rs merge 3 lớp, tracing init, error.rs | `agentwiki -p tests/fixture-app --dry-run` in DAG + config hiệu lực; `cargo clippy -D warnings` sạch |
| M1 | `backend/` trait + `DevinBackend` + `agent/runner.rs` + 1 spec `dir_summary` | chạy fixture → `.agentwiki/cache/*.json` có JSON hợp lệ; `calls.jsonl` có dòng tương ứng; quota chặn khi `daily_cap=1` |
| M2 | scanner đủ dùng + 6 research spec + fan-out per-domain | fixture → `ResearchContext` đầy đủ các report typed |
| M3 | editors + doctree + writer | ra đủ `1.Overview.md … 4.Deep-Exploration/*` trên fixture |
| M4 | verify mermaid + integrity + summary report | block mermaid lỗi cố ý trong fixture được sửa hoặc flag trong report |
| M5 | MockBackend + `pipeline_offline.rs` | `cargo test` green, không gọi CLI thật |
| M6 | `ClaudeBackend` + `CodexBackend` + agentic mode | `AGENTWIKI_E2E=1 cargo test -- --ignored` pass trên cả 3 backend |

## 13. Prompt templates — port từ đâu

Prompt của Litho nằm rải trong `src/generator/**` (Rust string). Bước đầu
của M2/M3: **extract prompt text** về `prompts/`, giữ instruction, thay
phần inject bằng `{{placeholder}}`. Render bằng helper nhỏ nội bộ
(`render(tmpl, &ctx_map)`); nếu sau này cần logic phức tạp thì đổi sang
`minijinja` — interface giữ nguyên. Ngôn ngữ output qua placeholder chung
("Write all prose in <language>. JSON keys stay in English."). Prompt load
ưu tiên file trên disk (`config.prompts_dir`), fallback `include_str!`.

## 14. Rủi ro & giảm thiểu

| Rủi ro | Giảm thiểu |
|---|---|
| Agent CLI tự ý dùng tool / sửa file | embedded mode chạy trong empty-cwd; agentic mode kèm chỉ dẫn read-only |
| Model trả text thay vì JSON | retry-with-error (mục 7); `tier=powerful` cho schema phức tạp |
| `devin -p` session overhead mỗi call | cache aggressive; gom nhiều câu hỏi/1 prompt khi hợp lý |
| Prompt quá dài (repo lớn) | per-dir summary trước, chỉ nhét insight có chọn lọc (như `boundary_code_limit` của Litho) |
| devin CLI đổi flag | lệnh gọi gom trong `build_cmd()` duy nhất mỗi backend; e2e smoke bắt lỗi sớm |
| "Free" model có thể vẫn đếm quota plan | daily_cap + calls.jsonl audit |

## 15. Quyết định mở (chốt trước khi M1)

1. Tên package: `agentwiki` (đổi được, chỉ là chuỗi trong Cargo.toml).
2. Model default efficient/powerful: đề xuất `devin:swe-2-medium` cả hai
   (Free). Cần chất lượng → `devin:swe-2-high`/`swe-2-max` cho powerful.
3. Ngôn ngữ docs mặc định: `en` (`--target-language vi` khi cần).
4. Giữ tên output files giống Litho (`1.Overview.md`…) — đề xuất giữ.
5. Repo lớn: giới hạn insight/prompt — đề xuất mặc định `code_insights_limit
   = 25`, chỉnh qua config.
