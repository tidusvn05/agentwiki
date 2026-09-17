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
| `codex` | Codex CLI | `codex:gpt-5-codex` |

Billing-related env vars (`ANTHROPIC_*`, `OPENAI_*`, `DEVIN_API*`, …) are
stripped from spawned children so calls stay on subscription auth, not API
billing.

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
agentwiki -p . -o docs/          # default: embedded mode, English docs
```

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

# Named profiles — `agentwiki <profile>` applies them:
[profiles.research]
skip_documentation = true
```

`agentwiki` and `agentwiki default` are equivalent — `default` is a
built-in profile, so it works even with no config file. Defining
`[profiles.default]` yourself overrides the built-in.

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
