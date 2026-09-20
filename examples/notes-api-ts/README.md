# notes-api-ts — a clean real run

A small layered REST-ish service in TypeScript. This is the **full tier**:
everything under `docs/` — nine generated Markdown files plus
`agentwiki.claims.json` — is real output from a `devin` (swe-2-medium)
pipeline run, committed exactly as CI would see it.

```text
src/
  index.ts  server.ts  models.ts  config.ts
  routes/     index.ts
  handlers/   tasks.ts, notes.ts, users.ts
  middleware/ auth.ts
  infra/      db.ts, cache.ts
db/schema.sql
docs/         1.Overview.md … 6.Database-Overview.md, 4.Deep-Exploration/
docs/agentwiki.claims.json          # real `devin` output (15 edges)
```

## The real claims — `agentwiki drift -p .`

```text
coverage: 14/15 claims checked (93%) — 1 unverifiable
  ok   14 confirmed (14 direct_evidence)
 info  1 unverifiable        infra/db.ts→schema.sql [kind_not_checkable]
result: 0 finding(s) would fail under --strict
```

This is what a **green project** looks like:

- The model claimed at file granularity (`index→server`, `server→routes`,
  `handlers/*→db`, `infra/*→config`, `middleware→config`…) and every one
  resolves to a real import — including `Composition` and `FunctionCall`
  claims, which are all import-checkable.
- The only `DataFlow` claim is `db.ts → db/schema.sql` — again the correct
  "no code-level reference" usage, so `kind_not_checkable` is honest, not
  noise.
- `filtered: u8_thin=2` — `tasks→models` and `users→models` are real edges
  missing from claims, but with a single import site each they stay under
  `min_undocumented_imports=3`. Single-site fan-in is treated as too thin
  to report.
- `--strict` exits 0 immediately: nothing to baseline.

## Resolver coverage on display

Extension-less specifiers (`"./server"`), parent traversal
(`"../infra/db"`), and a directory index (`"./routes/index"`) all resolve
through the JS/TS resolver — check `docs/agentwiki.claims.json` against
`src/` to see how each claim maps to a file.

## Try it

```sh
agentwiki drift -p . -v
agentwiki drift -p . --json | jq '.coverage, .counts'

# break a claim and watch the verdict change
#   e.g. set "from": "src/models.ts", "to": "src/server.ts"  → phantom
#   or   "src/infra"  → "src/server.ts"                      → reversed
```
