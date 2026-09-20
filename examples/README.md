# Drift-check examples

Two small projects used to verify `agentwiki drift` end to end. Each ships
a committed `docs/agentwiki.claims.json` — the same file the pipeline
writes next to the docs — so `drift` works here exactly like it does on a
bare CI checkout (no `.agentwiki/` needed).

| Example | Language | Story |
|---|---|---|
| [`taskman-py`](./taskman-py) | Python | **Anatomy of every verdict.** Real `devin` claims with a genuine hallucinated edge (`phantom`), plus a curated claims file that exercises all six finding classes. |
| [`notes-api-ts`](./notes-api-ts) | TypeScript | **A clean real run.** Full generated doc set + claims from `devin`: 14/15 claims confirmed against code, 93% coverage, zero warnings. |

## Quickstart

```sh
# from the repo root — no build config needed, each project carries an
# agentwiki.toml pointing drift at its committed claims file
agentwiki drift -p examples/taskman-py -v
agentwiki drift -p examples/notes-api-ts -v
```

Expected output is checked in next to the claims (`docs/expected-drift.txt`).

## Reading a report

Every claim lands in exactly one class:

| Class | Level | Meaning | `--strict` |
|---|---|---|---|
| `confirmed` | ok | import evidence found (direct, transitive, or containment) | – |
| `structural` | info | containment-shaped but no code evidence (e.g. parent→child) | – |
| `unverifiable` | info | can't be checked statically (`data_flow`, non-code or unknown endpoint, low language coverage) | – |
| `undocumented` | warn | real edge in code, missing from claims | baselined |
| `reversed` | warn | the code has the edge backwards | baselined |
| `phantom` | warn | claimed, but no evidence at all | baselined |

`coverage: N/M` counts checked claims (`confirmed`+`phantom`+`reversed`)
over all deduped claims. `filtered:` shows how many candidate
undocumented edges the noise filters suppressed.

## Things to try

```sh
# gate on new problems only
agentwiki drift -p examples/taskman-py --strict          # exit 1 (phantom)

# quiet CI after reviewing: record findings, then --strict passes
agentwiki drift -p examples/taskman-py --update-baseline
agentwiki drift -p examples/taskman-py --strict          # exit 0

# the full verdict matrix on one file (curated claims)
agentwiki drift -p examples/taskman-py --claims docs/claims-curated.json -v

# machine-readable report
agentwiki drift -p examples/notes-api-ts --json | jq .coverage
```

Edit a claim in `docs/agentwiki.claims.json` (flip a `from`/`to`, point it
at a file that never imports the target) and re-run — the verdict changes
deterministically, no LLM involved.
