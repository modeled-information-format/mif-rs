---
id: reference-mif-rs-m2-review-benchmark
type: semantic
created: '2026-07-04T00:00:00Z'
modified: '2026-07-04T00:00:00Z'
namespace: reference/benchmarks
title: M2 Review Performance Benchmark
tags:
  - reference
  - benchmarks
  - performance
temporal:
  '@type': TemporalMetadata
  validFrom: '2026-07-04T00:00:00Z'
  recordedAt: '2026-07-04T00:00:00Z'
  ttl: P1Y
ontology:
  '@type': OntologyReference
  id: mif-docs
  version: 1.0.0
  uri: https://mif-spec.dev/ontologies/mif-docs
entity:
  name: M2 Review Performance Benchmark
  entity_type: reference-doc
---

# M2 Review Performance Benchmark

Append-only record of manual `mif-rh-cli review` performance runs against the
real research-harness findings corpus, proving the PRD's M2 milestone.

## Target

**M2 (PRD):** `mif-rh-cli review` over the full real corpus (~4400 findings)
completes in under **300 seconds** (5 minutes) wall time.

## Method

Runs use the `bench-review` justfile recipe, which builds `mif-rh-cli` in
release mode, counts finding files (any-depth `**/findings/*.json` under `reports/`) in the
corpus, and times a default `review` (no `--strict`, no `--build-index`)
with `/usr/bin/time -p` from the corpus root:

```bash
just bench-review <disposable-corpus-copy>
```

- A relative corpus path resolves against the repository root (`just` runs
  recipes there), not the shell's current directory — pass an absolute path
  when unsure.
- The corpus is local and private; it is never checked in and this benchmark
  is **not** wired into CI. It exists to seed and refresh the results table
  below by hand.
- `review` **rewrites** `reports/<topic>/ontology-map.json` and
  `reports/_meta/` inside `CORPUS_DIR` — always run against a disposable
  copy of the corpus, never a pristine checkout.
- The recipe's finding count is a raw file count; the CLI's own summary line
  is the authoritative classification count. The two can differ when
  non-topic fixture files match the path pattern (e.g.
  `reports/_meta/sample-session/findings/*.json`).
- If the corpus copy lacks `scripts/check-relationship-targets.sh`, `review`
  skips the relationship-targets check; note it in the results row.

## Results

Append one row per run; never rewrite or delete prior rows.

| date | mif-rs commit | corpus (findings) | hardware | wall (s) | findings/sec | notes |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-07-04 | 60107bc | rht `mif-rh-m1` (4354) | Apple M4 Max, 128 GB, macOS 26.5 | 2.02 | 2155.4 | First seeded run. Review classified 4351 findings across 38 topics (the 3-file delta is `reports/_meta/sample-session` fixtures). Relationship-targets script absent from the copy, check skipped. |
