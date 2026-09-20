# agentwiki

Generate **C4-style architecture documentation** for any repository using an
already-authenticated agent CLI (`devin`, `claude`, `codex`) as the LLM —
no metered API key required.

AgentWiki is a CLI-agent-native rewrite of
[deepwiki-rs](https://github.com/sopaco/deepwiki-rs): instead of an
OpenAI-compatible HTTP API, every pipeline agent is a subprocess call to a
local agent CLI. If you can run the CLI, you can run agentwiki.

## What it produces

```
agentwiki.docs/
├── 1.Overview.md
├── 2.Architecture.md
├── 3.Workflow.md
├── 4.Deep-Exploration/<Domain>.md   # one doc per detected domain module
├── 5.Boundary-Interfaces.md
├── 6.Database-Overview.md
└── __AgentWiki_Summary__.md         # timings, calls, cache hits
```

A real self-generated example lives in [`docs/en/`](docs/en/) — agentwiki's
own architecture docs, produced by `agentwiki` itself
([`docs/vi/`](docs/vi/) for the Vietnamese version).

## Features

- **DAG pipeline** — research agents (dir summaries, system context, domain
  modules, relationships, boundary, database) feed compose agents that write
  the docs.
- **Content-hash cache** — each call is cached by `sha256(prompt‖model‖backend)`
  under `.agentwiki/cache/`. Interrupt any time; a rerun reuses everything
  that already succeeded.
- **Safe to interrupt** — `Ctrl-C` cancels cooperatively, kills in-flight CLI
  children, and exits `130`. A second `Ctrl-C` force-quits.
- **Live progress** — spinner bar shows `[done/total]` and which agents are
  running right now (auto-hidden when output is piped).
- **Quota + audit** — daily call cap (`300`/day default) and a
  `.agentwiki/calls.jsonl` audit log.
- **Retries + fallback** — schema validation failures retry with feedback;
  efficient-tier agents fall back to the powerful model.
- **Run lock** — a second concurrent run on the same repo fails fast
  (`.agentwiki/run.lock`, stale locks auto-reclaimed).
- **`agentwiki doctor`** — health check: CLI availability, running/orphaned
  agent processes, lock/quota/cache state, config sanity (`--fix` cleans up).

## Prerequisites

At least one agent CLI, installed and authenticated:

| Backend | CLI | Model string example |
|---|---|---|
| `devin` | [Devin CLI](https://devin.ai) | `devin:swe-2-medium` (default) |
| `claude` | Claude Code CLI | `claude:sonnet` |
| `codex` | Codex CLI | `codex:gpt-5.6-sol@high` |

Billing-related env vars (`ANTHROPIC_*`, `OPENAI_*`, `DEVIN_API*`, …) are
stripped from spawned children so calls stay on subscription auth, not API
billing.

The codex and claude backends accept an optional `@<effort>` suffix on the
model — `codex:<model>@<effort>` sets `model_reasoning_effort`
(`low medium high xhigh max ultra`), `claude:<model>@<effort>` maps to
`--effort` (`low medium high xhigh max`). A bare `<backend>:<model>` inherits
the CLI's configured default. A balanced pairing for doc generation:

```toml
[models]
efficient = "codex:gpt-5.6-sol@low"    # fan-out summaries + JSON extraction
powerful  = "codex:gpt-5.6-sol@high"   # architecture/workflow/deep-dive writing
```

## Install

### Prebuilt binary (no source checkout needed)

```sh
curl -fsSL https://raw.githubusercontent.com/tidusvn05/agentwiki/main/install.sh | sh
```

downloads the latest release binary straight to `~/.local/bin/agentwiki`
(Linux x86_64, macOS Intel & Apple Silicon; Windows binaries are on the
[Releases](https://github.com/tidusvn05/agentwiki/releases) page).

### cargo install

```sh
cargo install --git https://github.com/tidusvn05/agentwiki
```

### From source

```sh
git clone https://github.com/tidusvn05/agentwiki
cd agentwiki && cargo build --release
./target/release/agentwiki --help
```

## Quickstart

```sh
cd your-repo
agentwiki                            # auto-detect backend: devin → codex → claude
agentwiki devin                      # or pick a backend explicitly
agentwiki claude --target-language vi
agentwiki codex --target-language ja
```

Docs land in `./agentwiki.docs/`; cache and run state in `./.agentwiki/`.
Watch the progress bar; interrupt any time — rerunning resumes from the
cache for free.

Useful flags:

| Flag | Effect |
|---|---|
| `-p, --project-path` | repo to document (default `.`) |
| `-o, --output-path` | docs output dir (default `./agentwiki.docs`) |
| `--agentic` | let the agent read the repo itself instead of embedding code |
| `--target-language` | `en zh ja ko de fr ru vi` |
| `--model-efficient` / `--model-powerful` | `"<backend>:<model>"` per tier |
| `--max-parallels` | concurrent CLI calls (default `2`) |
| `--dry-run` | print resolved config + task DAG, exit |
| `--skip-research` | reuse `.agentwiki/research.json`, only recompose |
| `--skip-documentation` | stop after research |
| `--force-regenerate` / `--no-cache` | cache controls |
| `-v … -vvv` | log verbosity (`warn` default) |

### `agentwiki doctor`

Read-only environment check — run it before (or instead of debugging) a
failed pipeline:

```sh
agentwiki doctor          # report only
agentwiki doctor --fix    # also remove a stale run.lock + leftover tempfiles
```

It reports: agent CLIs on `PATH` (with versions, and which ones the
configured models require), running/orphaned agent processes, the
`.agentwiki/` state (run lock, quota usage, cache size, `research.json`,
last call), and project/output writability. Exit code is `1` when any
check fails, so it is usable in scripts.

### `agentwiki drift`

Read-only drift check: compares the generated `core_dependencies` claims
against a statically-extracted import graph (Rust / Python / JS-TS) and
reports `confirmed` / `structural` / `unverifiable` / `undocumented` /
`reversed` / `phantom` edges. No LLM calls, no writes to your tree except
`<internal>/drift.json`.

```sh
agentwiki drift                      # report only, exit 0
agentwiki drift -v                   # also list info-level edges
agentwiki drift --json               # machine-readable report on stdout
agentwiki drift --strict             # exit 1 on NEW phantom/reversed
agentwiki drift --update-baseline    # record current findings, exit 0
agentwiki drift --baseline ci-baseline.json   # override baseline path
agentwiki drift --max-depth 2        # tighter transitive check
agentwiki drift -o docs              # docs dir, for the claims search below
agentwiki drift --export-claims claims.json   # commit-able claims copy
```

The `coverage:` line shows how many claims were actually checked against
code (`confirmed`/`phantom`/`reversed`) versus skipped as `structural`/
`unverifiable` — it separates "docs match the code" from "couldn't check".

`.agentwiki/` is gitignored, so CI needs the claims committed: the
pipeline already writes `<output>/agentwiki.claims.json` next to the docs,
so committing the doc tree is enough. `drift` looks for claims in this
order: `--claims` → `[drift].claims_path` → `<internal>/research.json` →
`<output>/agentwiki.claims.json`. `--export-claims` remains for keeping
the file somewhere else.

Suggested CI rollout:

```yaml
# 1. observe — learn the noise level, never fail
- run: agentwiki drift
  continue-on-error: true
# 2. locally: `agentwiki drift --update-baseline`, commit the baseline
#    (claims ride along with the committed docs)
# 3. gate — fail only on NEW phantom/reversed findings
- run: agentwiki drift --strict
```

## Configuration

`agentwiki.toml` in the project (or `~/.config/agentwiki/config.toml` for
global defaults) — all keys optional:

```toml
target_language = "vi"        # en zh ja ko de fr ru vi
mode = "agentic"              # embedded | agentic
max_parallels = 4

[models]
efficient = "devin:swe-2-medium"
powerful  = "devin:swe-2-max"

[limits]
daily_cap       = 300
call_timeout_s  = 600
retry_attempts  = 3

[scan]
git_tracked_only = true       # only files in `git ls-files`
include_hidden   = false
include_tests    = false
excluded_dirs    = ["target", "node_modules", "vendor", "dist", ...]
excluded_files   = ["*.lock", ".env", "Cargo.lock", ...]

# `agentwiki drift` — all keys optional, the interesting ones:
[drift]
claims_path    = "claims.json"       # default: <internal>/research.json, else <output>/agentwiki.claims.json
baseline_path  = ".drift-baseline.json"   # default: .agentwiki-drift-baseline.json
ignore_nodes   = ["src/generated"]   # never flag undocumented edges here
max_transitive_depth = 3             # hops for `confirmed transitive`

# Named profiles — `agentwiki <profile>` applies them:
[profiles.research]
skip_documentation = true
```

`agentwiki` and `agentwiki default` are equivalent — `default`, `devin`,
`claude`, `codex` are built-in profiles, so they work even with no config
file. A backend profile just sets `[models]` to that CLI's default pair
(`claude` → `sonnet@low`/`sonnet@high`, `codex` → `gpt-5.6-sol@low`/`@high`,
`devin` → `swe-2-medium`); defining `[profiles.<name>]` yourself overrides
the built-in.

When no model is configured anywhere — no `[models]` in config, profile, or
`--model-*` flags — agentwiki picks the first agent CLI on `PATH`, in the
order **devin → codex → claude**.

### `embedded` vs `agentic` mode

| | `embedded` (default) | `agentic` |
|---|---|---|
| Context | scanner embeds code + materials into the prompt | agent reads the repo itself with its own tools |
| Agent cwd | empty dir (`.agentwiki/empty-cwd`) — repo invisible | the project root |
| Prompt size | capped by `limits.materials_char_cap` (192 KB) | no cap — agent explores as needed |
| Speed/cost | predictable, faster | slower — agents spend turns reading files |
| Requirements | any CLI | CLI must have working file tools (`devin`, `claude`, `codex` all do) |

Use **embedded** for routine runs — deterministic context, no wandering.
Use **agentic** (`--agentic` or `mode = "agentic"`) for large or tangled
codebases where the embedded excerpts feel too shallow and the agent
should chase references itself.

## How it works

`Preprocess → Research → Compose → Write → Verify`

- **Preprocess**: deterministic walk (git-tracked filter, exclusions,
  importance scoring) — no LLM.
- **Research**: `dir_summary` fans out per directory, then typed agents
  (system context, domain modules, relationships, boundary, database,
  architecture, workflow, key-module) with lenient-JSON validation.
- **Compose**: LLM editors + deterministic renderers → the doc tree.
- **Verify**: file integrity + mermaid checks + summary report.

Design notes: [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md).

## Development

```sh
cargo build          # build
cargo test           # offline tests (MockBackend — no real CLI needed)
cargo clippy --all-targets
AGENTWIKI_E2E=1 cargo test --test e2e_real_cli   # real CLI calls
```

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Bug reports and PRs welcome.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

Portions ported from [deepwiki-rs](https://github.com/sopaco/deepwiki-rs)
(MIT © Sopaco) — see [NOTICE](NOTICE).
