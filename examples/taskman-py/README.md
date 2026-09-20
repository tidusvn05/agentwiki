# taskman-py — anatomy of every verdict

A small layered CLI task manager in Python. Two layers call down
(`main → cli → handlers → storage → core`), a `services/` layer sits
parallel to it, and `db/schema.sql` is a pure data contract.

```text
src/
  main.py  cli.py  handlers.py  storage.py  models.py
  core/      db.py, naming.py        # shared primitives
  services/  sync.py, mail.py, report.py
db/schema.sql                        # tasks table — no code references it
docs/agentwiki.claims.json           # real `devin` output (11 edges)
docs/claims-curated.json             # hand-built: hits every finding class
```

Commands below assume `cd examples/taskman-py` (or swap `-p .` for
`-p examples/taskman-py` from the repo root).

## The real claims — `agentwiki drift -p .`

```text
coverage: 9/11 claims checked (82%) — 2 unverifiable
  ok   8 confirmed (8 direct_evidence)
 info  2 unverifiable        storage→schema.sql [kind_not_checkable],
                            handwritten.claims.json→src [unknown_endpoint]
 warn  1 phantom            report.py→core/db.py
result: 1 finding(s) would fail under --strict
```

Highlights:

- **`phantom: src/services/report.py -> src/core/db.py`** — the model
  claimed "report rendering queries tasks via the shared DB connection
  factory", but `report.py` only imports `core.naming` and `models`. A
  real hallucination, caught without an LLM.
- **`storage.py -> db/schema.sql` [data_flow → unverifiable]** — correct
  use of `DataFlow`: the schema couples storage to the DB with no
  code-level reference, so there is nothing to check. (The prompt now
  reserves `DataFlow` for exactly this case.)
- **`handwritten.claims.json -> src` [unknown_endpoint]** — the model
  noticed a stray claims file during the run and claimed against it. The
  file isn't shipped here, so the endpoint can't resolve — a free demo of
  `unknown_endpoint` handling.
- `filtered: u8_thin=4` — four real edges (`sync→db`, `mail→db`,
  `report→naming`, `report→models`) stayed under
  `min_undocumented_imports=3`, so no undocumented noise.

## The curated claims — every class in one run

```sh
agentwiki drift -p . --claims docs/claims-curated.json -v
```

```text
coverage: 8/11 claims checked (73%) — 2 unverifiable
  ok   6 confirmed (3 containment, 2 direct_evidence, 1 transitive)
 info  1 structural          src→src/services (containment, no evidence)
 info  2 unverifiable        db→src [language_coverage], src→db [kind_not_checkable]
 warn  1 phantom             services→main.py
 warn  1 reversed            cli.py→main.py  (code has main→cli)
 warn  1 undocumented        services→core   (4 import sites)
result: 2 finding(s) would fail under --strict
```

Notable mechanics on display:

- **`structural`** — `src → src/services` is containment-shaped but no
  file under the `src` node imports `services`, so it is "plausible by
  layout, unproven by code" — never a failure.
- **`confirmed containment`** — `src/services → src` is proven by
  `sync.py→storage.py` and `report.py→models.py` (both targets live in the
  `src` node); `src → src/core` by `storage.py→core/db.py`.
- **`undocumented`** — `services → core` has 4 independent import sites
  and no deep claims path, so it surfaces even though the docs mention
  both ends.
- **`reversed`** — claiming `cli.py → main.py` trips on the real
  `main.py → cli.py` edge (evidence line shows `src/main.py:2`).

## Strict + baseline

```sh
agentwiki drift -p . --strict            # exit 1 — phantom (+reversed in curated)
agentwiki drift -p . --update-baseline   # writes .agentwiki-drift-baseline.json
agentwiki drift -p . --strict            # exit 0 — everything is now "known"
```

The baseline stores only gating ids (`phantom`/`reversed`/`undocumented`);
`confirmed`/`structural`/`unverifiable` verdicts never enter it.
