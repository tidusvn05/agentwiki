# Contributing

Thanks for your interest in agentwiki.

## Setup

```sh
git clone https://github.com/tidusvn05/agentwiki
cd agentwiki
cargo build && cargo test
```

All tests run offline via `MockBackend` — no agent CLI or network needed.
`AGENTWIKI_E2E=1 cargo test --test e2e_real_cli` runs real `devin`/`claude`/
`codex` calls (only if you have those CLIs authenticated).

## Conventions

- `cargo clippy --all-targets` must be clean (`-D warnings` in CI).
- No `unwrap()`/`expect()` outside tests.
- New backend = one file in `src/backend/` + one arm in `for_kind()`.
- New DAG agent = one `AgentSpec` in `src/agent/registry.rs` + a prompt
  template in `prompts/` (+ a typed report in `src/agent/reports/` when it
  returns JSON).
- Keep prompt/LLM behavior deterministic where possible; prefer adding to
  the typed-report layer over free-form text.

## Pull requests

Small, focused PRs. Describe *why*, not just *what*. If your change affects
the pipeline or cache format, bump `SCHEMA_VERSION` in `src/cache.rs` so old
cache entries invalidate cleanly.
