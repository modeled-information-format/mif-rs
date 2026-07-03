# mif-frontmatter

Markdown frontmatter <-> JSON-LD lossless round-trip for the [MIF (Modeled Information Format)](https://mif-spec.dev) ecosystem.

Ports the canonical `mif_convert.py` reference converter (from the `MIF` spec
repository) to Rust. Per the MIF v1.0 specification, a concept file is YAML
frontmatter followed by a markdown body; the frontmatter is source of truth
(Invariant 2) and JSON-LD is a *derived* projection that must be lossless on
a `markdown -> json-ld -> markdown` round trip for all conformance-level
data (Invariant 4).

The core functions mirror that pipeline directly: [`parse_markdown`] splits
a concept file into frontmatter and body, [`serialize_markdown`] puts them
back together in canonical form (keys ordered per [`FRONTMATTER_ORDER`],
extras appended), [`md_to_jsonld`]/[`jsonld_to_md`] project between the
parsed frontmatter and its derived JSON-LD, and [`roundtrip_lossless`] drives
the full pipeline end to end and fails with a [`FrontmatterError::RoundTripDrift`]
if the recovered markdown doesn't match the canonical serialization of the
original.

## `FrontmatterShape`

A document's frontmatter can express its `@id`/`@type`/`conceptType`
identity two different ways, and the two are genuinely ambiguous to tell
apart from a projected JSON-LD document alone — a v1.0 `id: foo` shorthand
and an already-literal `@id: urn:mif:foo` key produce the identical `@id`
string once projected. `FrontmatterShape` names the two conventions so
callers of `jsonld_to_md` can say which one to reconstruct:

- `V1Canonical` — bare `id`/`type` frontmatter fields project to
  `@id`/`conceptType` (the v1.0 authoring convention `mif_convert.py` and
  this crate's own examples use).
- `PreProjected` — frontmatter already carries `@context`/`@type`/`@id`/
  `conceptType` directly as literal keys (e.g. `research-harness-template`'s
  Level-3 report documents), passed through verbatim with no bare-id-to-URN
  projection.

`md_to_jsonld`'s input is a frontmatter mapping, which unambiguously reveals
its own shape by whether it already contains a literal `@id` key, so it
detects this automatically via an internal `detect_shape` check;
`jsonld_to_md` cannot, so it takes `shape` as an explicit parameter instead
of guessing.

## Constants and errors

`CONTEXT_URL` is the JSON-LD `@context` URL emitted by `md_to_jsonld`.
`FRONTMATTER_ORDER` is the canonical frontmatter key order used for
deterministic, lossless serialization — keys not in the list are appended
afterward in their original encounter order. `FrontmatterError` (via
`thiserror`) covers every failure mode along the pipeline, from a missing
frontmatter block through YAML/JSON conversion failures to
`RoundTripDrift`, and implements `mif_problem::ToProblem` for RFC 9457
`application/problem+json` reporting.

## Known deviations from the Python reference

- `PyYAML` resolves unquoted YAML timestamps into typed `datetime`/`date`
  objects, so `mif_convert.py` has to explicitly re-stringify them.
  `serde_norway` has no such implicit timestamp resolution — every scalar
  deserializes as a plain string already — so this crate has no equivalent
  step.
- `mif_convert.py`'s `jsonld_to_md` only recovers a fixed list of
  passthrough fields, silently dropping any other frontmatter key on the
  full round trip even though `serialize_markdown` alone preserves it. This
  crate deliberately does **not** reproduce that limitation:
  `md_to_jsonld`/`jsonld_to_md` pass every frontmatter/JSON-LD key through
  generically — `FRONTMATTER_ORDER` governs serialization *order* only, not
  which keys survive. This matches the canonical `mif.schema.json`, whose
  root object schema does not set `additionalProperties: false`, so
  unrecognized top-level keys are already spec-legal; silently dropping
  them was a bug in the reference converter, not a behavior worth
  preserving.
- Where Python would raise an unhandled exception on malformed input (e.g.
  frontmatter that parses to a YAML scalar instead of a mapping, or an `id`
  field that isn't a string), this crate returns a `FrontmatterError`
  variant instead, since library code here may not panic.

## License

MIT
