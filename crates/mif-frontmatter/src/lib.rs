//! Markdown frontmatter <-> JSON-LD projection for the MIF (Modeled
//! Information Format) ecosystem.
//!
//! Ports the canonical `mif_convert.py` reference converter (from the `MIF`
//! spec repository) to Rust. Per the MIF v1.0 specification, a concept file
//! is YAML frontmatter followed by a markdown body; the frontmatter is
//! source of truth (Invariant 2) and JSON-LD is a *derived* projection that
//! must be lossless on a `markdown -> json-ld -> markdown` round trip for
//! all conformance-level data (Invariant 4).
//!
//! # Known deviations from the Python reference
//!
//! - `PyYAML` resolves unquoted YAML timestamps (`2026-01-15T10:30:00Z`) into
//!   typed `datetime`/`date` objects, so `mif_convert.py` has to explicitly
//!   re-stringify them (`stringify_datetimes`). `serde_norway` has no such
//!   implicit timestamp resolution — every scalar deserializes as a plain
//!   string already — so this crate has no equivalent step.
//! - `mif_convert.py`'s `jsonld_to_md` only recovers a fixed list of
//!   passthrough fields from a JSON-LD document, silently dropping any other
//!   frontmatter key on the full `md -> json-ld -> md` pipeline even though
//!   `serialize_markdown` alone preserves it (real corpora hit this: a
//!   document that adds its own frontmatter fields beyond the ones
//!   `mif_convert.py` happens to name loses them on round trip). This crate
//!   deliberately **does not** reproduce that limitation: [`md_to_jsonld`]
//!   and [`jsonld_to_md`] pass every frontmatter/JSON-LD key through
//!   generically — [`FRONTMATTER_ORDER`] governs serialization *order* only,
//!   not which keys survive. This is consistent with the canonical
//!   `mif.schema.json`, whose root object schema does not set
//!   `additionalProperties: false`, so unrecognized top-level keys are
//!   already spec-legal; silently dropping them was a bug in the reference
//!   converter, not a behavior worth preserving.
//! - Where Python would raise an unhandled exception on malformed input
//!   (e.g. frontmatter that parses to a YAML scalar instead of a mapping,
//!   or an `id` field that isn't a string), this crate returns a
//!   [`FrontmatterError`] variant instead, since library code here may not
//!   panic.

use mif_problem::{
    Applicability, CodeAction, ProblemDetails, ProblemMeta, SuggestedFix, ToProblem,
};
use serde_norway::{Mapping, Value};

/// The JSON-LD `@context` URL emitted by [`md_to_jsonld`].
pub const CONTEXT_URL: &str = "https://mif-spec.dev/schema/context.jsonld";

/// Canonical frontmatter key order for deterministic, lossless
/// serialization. Keys not in this list are appended after it, in the
/// order they were first encountered.
pub const FRONTMATTER_ORDER: &[&str] = &[
    "id",
    "type",
    "memoryType",
    "created",
    "modified",
    "namespace",
    "title",
    "summary",
    "properties",
    "compressedAt",
    "tags",
    "aliases",
    "temporal",
    "provenance",
    "embedding",
    "relationships",
    "citations",
    "documents",
    "entities",
    "ontology",
    "entity",
    "extensions",
];

/// Parsed YAML frontmatter, preserving source key order.
pub type Frontmatter = Mapping;

/// Errors from parsing, serializing, or projecting MIF frontmatter.
#[derive(Debug, thiserror::Error)]
pub enum FrontmatterError {
    /// The input did not start with a `---\n...\n---` YAML frontmatter
    /// block.
    #[error("no YAML frontmatter block found")]
    MissingFrontmatter,
    /// The frontmatter block was not valid YAML.
    #[error("failed to parse frontmatter as YAML: {source}")]
    Yaml {
        /// The underlying parse error.
        #[source]
        source: serde_norway::Error,
    },
    /// The frontmatter block parsed as YAML but was not a mapping (e.g. a
    /// bare scalar or sequence). Python's reference tool has no equivalent
    /// check and would fail later with an unhandled exception instead.
    #[error("frontmatter did not parse to a YAML mapping")]
    NotAMapping,
    /// Failed to serialize frontmatter back to YAML.
    #[error("failed to serialize frontmatter to YAML: {source}")]
    YamlSerialize {
        /// The underlying serialization error.
        #[source]
        source: serde_norway::Error,
    },
    /// Failed to convert a YAML value into its JSON-LD representation.
    #[error("failed to convert field '{field}' to JSON: {source}")]
    JsonConversion {
        /// The frontmatter field being converted.
        field: String,
        /// The underlying conversion error.
        #[source]
        source: serde_json::Error,
    },
    /// Failed to convert a JSON-LD value back into its YAML representation.
    #[error("failed to convert field '{field}' to YAML: {source}")]
    YamlConversion {
        /// The JSON-LD field being converted.
        field: String,
        /// The underlying conversion error.
        #[source]
        source: serde_norway::Error,
    },
    /// A field expected to be a string was not one.
    #[error("field '{field}' is not a string")]
    FieldNotAString {
        /// The offending field.
        field: String,
    },
    /// The JSON-LD input was not a JSON object.
    #[error("JSON-LD input is not a JSON object")]
    JsonNotAnObject,
    /// Failed to serialize or re-parse JSON-LD while simulating an on-disk
    /// projection round trip.
    #[error("failed to round-trip JSON-LD through JSON: {source}")]
    JsonRoundTrip {
        /// The underlying JSON error.
        #[source]
        source: serde_json::Error,
    },
    /// The `markdown -> json-ld -> markdown` round trip was not lossless.
    #[error("round-trip drift: expected {expected:?}, recovered {recovered:?}")]
    RoundTripDrift {
        /// The canonical serialization of the original frontmatter + body.
        expected: String,
        /// The canonical serialization recovered after the round trip.
        recovered: String,
    },
    /// A [`FrontmatterShape`] name was not recognized.
    #[error("unknown frontmatter shape: {0:?} (must be \"v1-canonical\" or \"pre-projected\")")]
    UnknownShape(String),
}

impl FrontmatterError {
    const fn meta(&self) -> ProblemMeta {
        match self {
            Self::MissingFrontmatter => ProblemMeta {
                slug: "missing-frontmatter",
                version: "v1",
                title: "No YAML frontmatter block found",
                status: 422,
                exit_code: 2,
            },
            Self::Yaml { .. } => ProblemMeta {
                slug: "invalid-frontmatter-yaml",
                version: "v1",
                title: "Frontmatter block is not valid YAML",
                status: 422,
                exit_code: 2,
            },
            Self::NotAMapping => ProblemMeta {
                slug: "frontmatter-not-a-mapping",
                version: "v1",
                title: "Frontmatter did not parse to a YAML mapping",
                status: 422,
                exit_code: 2,
            },
            Self::YamlSerialize { .. } => ProblemMeta {
                slug: "yaml-serialization-failure",
                version: "v1",
                title: "Internal error serializing frontmatter to YAML",
                status: 500,
                exit_code: 1,
            },
            Self::JsonConversion { .. } => ProblemMeta {
                slug: "field-json-conversion-failure",
                version: "v1",
                title: "Frontmatter field could not be converted to JSON",
                status: 422,
                exit_code: 2,
            },
            Self::YamlConversion { .. } => ProblemMeta {
                slug: "field-yaml-conversion-failure",
                version: "v1",
                title: "JSON-LD field could not be converted to YAML",
                status: 422,
                exit_code: 2,
            },
            Self::FieldNotAString { .. } => ProblemMeta {
                slug: "field-not-a-string",
                version: "v1",
                title: "Expected field was not a string",
                status: 422,
                exit_code: 2,
            },
            Self::JsonNotAnObject => ProblemMeta {
                slug: "jsonld-not-an-object",
                version: "v1",
                title: "JSON-LD input is not a JSON object",
                status: 422,
                exit_code: 2,
            },
            Self::JsonRoundTrip { .. } => ProblemMeta {
                slug: "json-roundtrip-failure",
                version: "v1",
                title: "Internal error round-tripping JSON-LD through JSON",
                status: 500,
                exit_code: 1,
            },
            Self::RoundTripDrift { .. } => ProblemMeta {
                slug: "roundtrip-drift",
                version: "v1",
                title: "Markdown -> JSON-LD -> markdown round trip was not lossless",
                status: 422,
                exit_code: 4,
            },
            Self::UnknownShape(_) => ProblemMeta {
                slug: "unknown-frontmatter-shape",
                version: "v1",
                title: "Unknown frontmatter shape",
                status: 400,
                exit_code: 2,
            },
        }
    }
}

impl ToProblem for FrontmatterError {
    fn to_problem(&self) -> ProblemDetails {
        let unspecified_internal = || {
            (
                SuggestedFix::new(
                    "This indicates a bug in mif-frontmatter. Report it upstream.",
                    Applicability::Unspecified,
                ),
                CodeAction::new(
                    "File a bug against mif-frontmatter",
                    "quickfix",
                    Applicability::Unspecified,
                ),
            )
        };
        let correctable_input = |suggestion: &str| {
            (
                SuggestedFix::new(suggestion.to_string(), Applicability::MaybeIncorrect),
                CodeAction::new(
                    "Fix the input document",
                    "quickfix",
                    Applicability::MaybeIncorrect,
                ),
            )
        };

        let (fix, action) = match self {
            Self::MissingFrontmatter => correctable_input(
                "Add a `---\\n...\\n---` YAML frontmatter block to the top of the document.",
            ),
            Self::Yaml { .. } => correctable_input("Fix the YAML syntax error in the frontmatter."),
            Self::NotAMapping => correctable_input(
                "Make the frontmatter block a YAML mapping, not a scalar or sequence.",
            ),
            Self::JsonConversion { .. }
            | Self::YamlConversion { .. }
            | Self::FieldNotAString { .. } => {
                correctable_input("Correct the offending field's type in the document.")
            },
            Self::JsonNotAnObject => {
                correctable_input("Supply a JSON-LD document whose top level is a JSON object.")
            },
            Self::RoundTripDrift { .. } => correctable_input(
                "The document uses a frontmatter key or structure that does not survive the \
                 markdown -> JSON-LD -> markdown round trip; simplify it or report the drift \
                 upstream if it should be supported.",
            ),
            Self::UnknownShape(_) => {
                correctable_input("Use \"v1-canonical\" or \"pre-projected\".")
            },
            Self::YamlSerialize { .. } | Self::JsonRoundTrip { .. } => unspecified_internal(),
        };
        self.meta()
            .into_details(env!("CARGO_PKG_NAME"), self.to_string())
            .with_suggested_fix(fix)
            .with_code_action(action)
    }
}

/// Splits a concept file into (frontmatter, body).
///
/// Mirrors `mif_convert.py`'s `FRONTMATTER_RE` (`^---\n(.*?)\n---\n?(.*)$`,
/// `re.DOTALL`): the frontmatter block runs from just after the opening
/// `---\n` to the first subsequent `\n---`, and one following newline (if
/// present) is consumed before the body begins.
///
/// # Errors
///
/// Returns [`FrontmatterError::MissingFrontmatter`] if `md_text` does not
/// start with `---\n` or contains no closing `\n---` delimiter, or
/// [`FrontmatterError::Yaml`]/[`FrontmatterError::NotAMapping`] if the
/// frontmatter block is not a valid YAML mapping.
///
/// # Examples
///
/// ```
/// let md = "---\nid: x\ntype: semantic\n---\n\nBody text.\n";
/// let (frontmatter, body) = mif_frontmatter::parse_markdown(md).unwrap();
/// assert_eq!(frontmatter.get("id").and_then(|v| v.as_str()), Some("x"));
/// assert_eq!(body, "\nBody text.\n");
/// ```
pub fn parse_markdown(md_text: &str) -> Result<(Frontmatter, String), FrontmatterError> {
    let after_open = md_text
        .strip_prefix("---\n")
        .ok_or(FrontmatterError::MissingFrontmatter)?;
    let close_pos = after_open
        .find("\n---")
        .ok_or(FrontmatterError::MissingFrontmatter)?;
    let yaml_text = &after_open[..close_pos];
    let rest = &after_open[close_pos + "\n---".len()..];
    let body = rest.strip_prefix('\n').unwrap_or(rest);

    let value: Value =
        serde_norway::from_str(yaml_text).map_err(|source| FrontmatterError::Yaml { source })?;
    let frontmatter = match value {
        Value::Null => Mapping::new(),
        Value::Mapping(m) => m,
        _ => return Err(FrontmatterError::NotAMapping),
    };
    Ok((frontmatter, body.to_string()))
}

/// Returns `frontmatter` reordered per [`FRONTMATTER_ORDER`], with any
/// remaining keys appended afterward in their original encounter order.
#[must_use]
fn ordered_frontmatter(frontmatter: &Frontmatter) -> Mapping {
    let mut ordered = Mapping::new();
    for key in FRONTMATTER_ORDER {
        let value_key = Value::String((*key).to_string());
        if let Some(value) = frontmatter.get(&value_key) {
            ordered.insert(value_key, value.clone());
        }
    }
    for (key, value) in frontmatter {
        if !ordered.contains_key(key) {
            ordered.insert(key.clone(), value.clone());
        }
    }
    ordered
}

/// Serializes frontmatter + body back into a concept file in canonical form.
///
/// Frontmatter keys are ordered per [`FRONTMATTER_ORDER`] (extras
/// appended), and the body has leading blank lines trimmed and exactly one
/// trailing newline.
///
/// # Errors
///
/// Returns [`FrontmatterError::YamlSerialize`] if the frontmatter cannot be
/// serialized to YAML.
///
/// # Examples
///
/// ```
/// let (frontmatter, body) =
///     mif_frontmatter::parse_markdown("---\nid: x\n---\n\nBody.\n").unwrap();
/// let text = mif_frontmatter::serialize_markdown(&frontmatter, &body).unwrap();
/// assert_eq!(text, "---\nid: x\n---\n\nBody.\n");
/// ```
pub fn serialize_markdown(
    frontmatter: &Frontmatter,
    body: &str,
) -> Result<String, FrontmatterError> {
    let ordered = ordered_frontmatter(frontmatter);
    let yaml_text = serde_norway::to_string(&Value::Mapping(ordered))
        .map_err(|source| FrontmatterError::YamlSerialize { source })?;
    let yaml_text = yaml_text.trim();
    let body = body.trim_start_matches('\n').trim_end();
    Ok(format!("---\n{yaml_text}\n---\n\n{body}\n"))
}

/// Frontmatter keys with their own special JSON-LD mapping in
/// [`md_to_jsonld`] (`id` -> `@id`, `type` -> `conceptType`); every other
/// frontmatter key passes through under its own name.
const FRONTMATTER_SPECIAL_KEYS: &[&str] = &["id", "type"];

/// JSON-LD keys unconditionally excluded from the generic pass-through in
/// [`jsonld_to_md`] under [`FrontmatterShape::V1Canonical`]:
/// `@context`/`@type`/`@id`/`conceptType`/`content` get their own special
/// frontmatter mapping (or are JSON-LD-only framing), and `timestamp` is
/// always a derived-only mirror of `created`/`modified` computed by
/// [`md_to_jsonld`] — writing it back to frontmatter would invent a field
/// the original document never had. `description` is deliberately **not**
/// listed here: unlike `timestamp`, it is only a derived mirror when the
/// frontmatter had a `summary` field; when it didn't, `description` is a
/// genuine pass-through key that `md_to_jsonld`'s generic loop carried
/// over verbatim, and must round-trip like any other field. See
/// [`jsonld_to_md`]'s handling of the `summary` key for how that
/// distinction is made.
const JSONLD_SPECIAL_KEYS: &[&str] = &[
    "@context",
    "@type",
    "@id",
    "conceptType",
    "content",
    "timestamp",
];

/// JSON-LD keys excluded from the generic pass-through in [`jsonld_to_md`]
/// under [`FrontmatterShape::PreProjected`]: only `content` (consumed into
/// the body). `@context`/`@type`/`@id`/`conceptType` pass through as literal
/// frontmatter keys, since this shape's frontmatter already carries them
/// directly. Unlike [`FrontmatterShape::V1Canonical`], `md_to_jsonld` never
/// synthesizes `timestamp`/`description` mirrors for this shape (see its
/// doc comment), so if they appear in `jsonld` here they are genuine
/// pass-through fields, not derived-only ones, and must round-trip like any
/// other key.
const PRE_PROJECTED_EXCLUDED_KEYS: &[&str] = &["content"];

/// Which convention a document's frontmatter uses to express its `@id`/
/// `@type`/`conceptType` identity.
///
/// The two conventions are genuinely ambiguous to distinguish from a JSON-LD
/// document alone (a v1.0 `id: foo` shorthand and an already-`@id:
/// urn:mif:foo` literal key both produce the identical `@id` string in the
/// projected JSON-LD), so [`jsonld_to_md`] requires the caller to say which
/// one to reconstruct; [`md_to_jsonld`]'s *input* is a frontmatter
/// [`Mapping`], which unambiguously reveals its own shape by whether it
/// already contains a literal `@id` key, so it detects this automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrontmatterShape {
    /// Bare `id`/`type` frontmatter fields project to `@id`/`conceptType`
    /// (the v1.0 canonical convention `mif_convert.py` and this crate's own
    /// examples use): `id: foo` becomes `@id: urn:mif:foo`.
    V1Canonical,
    /// Frontmatter already carries `@context`/`@type`/`@id`/`conceptType`
    /// directly as literal keys (e.g. `research-harness-template`'s Level-3
    /// report documents) — passed through verbatim, with no
    /// bare-id-to-URN projection and no `id`/`type` shorthand keys.
    PreProjected,
}

impl TryFrom<&str> for FrontmatterShape {
    type Error = FrontmatterError;

    /// Parses a shape name: `"v1-canonical"` or `"pre-projected"`. The
    /// single parser both `mif-cli` and `mif-mcp` call, so a caller-facing
    /// shape argument is validated identically everywhere rather than each
    /// binary hand-rolling its own string match.
    ///
    /// # Errors
    ///
    /// Returns [`FrontmatterError::UnknownShape`] for any other string.
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "v1-canonical" => Ok(Self::V1Canonical),
            "pre-projected" => Ok(Self::PreProjected),
            other => Err(FrontmatterError::UnknownShape(other.to_string())),
        }
    }
}

/// Detects which [`FrontmatterShape`] `frontmatter` uses, by whether it
/// already contains a literal `@id` key.
#[must_use]
fn detect_shape(frontmatter: &Frontmatter) -> FrontmatterShape {
    if frontmatter.get("@id").is_some() {
        FrontmatterShape::PreProjected
    } else {
        FrontmatterShape::V1Canonical
    }
}

/// Projects frontmatter + body into a derived JSON-LD document.
///
/// # Errors
///
/// Returns [`FrontmatterError::FieldNotAString`] if `id` is present but not
/// a string, or [`FrontmatterError::JsonConversion`] if a frontmatter field
/// cannot be converted to JSON.
///
/// # Examples
///
/// ```
/// use serde_norway::Mapping;
///
/// let mut frontmatter = Mapping::new();
/// frontmatter.insert("id".into(), "x".into());
/// frontmatter.insert("type".into(), "semantic".into());
/// let jsonld = mif_frontmatter::md_to_jsonld(&frontmatter, "Body.").unwrap();
/// assert_eq!(jsonld["@id"], "urn:mif:x");
/// assert_eq!(jsonld["content"], "Body.");
/// ```
pub fn md_to_jsonld(
    frontmatter: &Frontmatter,
    body: &str,
) -> Result<serde_json::Value, FrontmatterError> {
    let shape = detect_shape(frontmatter);
    let mut jsonld = serde_json::Map::new();

    if shape == FrontmatterShape::V1Canonical {
        jsonld.insert("@context".to_string(), serde_json::json!(CONTEXT_URL));
        jsonld.insert("@type".to_string(), serde_json::json!("Concept"));

        if let Some(id_value) = frontmatter.get("id") {
            let id_str = id_value
                .as_str()
                .ok_or_else(|| FrontmatterError::FieldNotAString {
                    field: "id".to_string(),
                })?;
            jsonld.insert(
                "@id".to_string(),
                serde_json::json!(format!("urn:mif:{id_str}")),
            );
        }
        if let Some(type_value) = frontmatter.get("type") {
            let converted = yaml_value_to_json("type", type_value)?;
            jsonld.insert("conceptType".to_string(), converted);
        }
    }

    for (key, value) in frontmatter {
        let Some(key_str) = key.as_str() else {
            continue;
        };
        if shape == FrontmatterShape::V1Canonical && FRONTMATTER_SPECIAL_KEYS.contains(&key_str) {
            continue;
        }
        jsonld.insert(key_str.to_string(), yaml_value_to_json(key_str, value)?);
    }

    // The `timestamp`/`description` OKF-recommended mirror fields are
    // synthesized only for V1Canonical: that shape's frontmatter never
    // authors them directly (they aren't in FRONTMATTER_ORDER), so deriving
    // them from created/modified/summary is safe. Synthesizing them
    // unconditionally for PreProjected would silently clobber a literal
    // `timestamp`/`description` frontmatter key that shape's generic
    // pass-through already carried into `jsonld` above — PreProjected's
    // whole design is "pass everything through, derive nothing."
    if shape == FrontmatterShape::V1Canonical {
        let timestamp = frontmatter
            .get("modified")
            .or_else(|| frontmatter.get("created"));
        jsonld.insert(
            "timestamp".to_string(),
            match timestamp {
                Some(value) => yaml_value_to_json("timestamp", value)?,
                None => serde_json::Value::Null,
            },
        );
        if let Some(summary) = frontmatter.get("summary") {
            jsonld.insert(
                "description".to_string(),
                yaml_value_to_json("summary", summary)?,
            );
        }
    }

    jsonld.insert("content".to_string(), serde_json::json!(body.trim()));
    Ok(serde_json::Value::Object(jsonld))
}

/// Recovers (frontmatter, body) from a derived JSON-LD document, projecting
/// the identity fields (`@id`/`conceptType`) per `shape`.
///
/// The two [`FrontmatterShape`]s are genuinely ambiguous to tell apart from
/// `jsonld` alone (see [`FrontmatterShape`]'s docs), so `shape` must be
/// supplied explicitly rather than guessed. [`roundtrip_lossless`] detects
/// it from the original markdown automatically; a caller with only a raw
/// JSON-LD document to project (no original frontmatter to consult) should
/// pass [`FrontmatterShape::V1Canonical`], the MIF v1.0 authoring
/// convention, unless it specifically wants the pre-projected form.
///
/// # Errors
///
/// Returns [`FrontmatterError::JsonNotAnObject`] if `jsonld` is not a JSON
/// object, [`FrontmatterError::FieldNotAString`] if `@id` is present but
/// not a string, or [`FrontmatterError::YamlConversion`] if a field cannot
/// be converted back to YAML.
///
/// # Examples
///
/// ```
/// use mif_frontmatter::FrontmatterShape;
///
/// let jsonld = serde_json::json!({
///     "@id": "urn:mif:x",
///     "conceptType": "semantic",
///     "content": "Body.",
/// });
/// let (frontmatter, body) =
///     mif_frontmatter::jsonld_to_md(&jsonld, FrontmatterShape::V1Canonical).unwrap();
/// assert_eq!(frontmatter.get("id").and_then(|v| v.as_str()), Some("x"));
/// assert_eq!(body, "Body.");
/// ```
pub fn jsonld_to_md(
    jsonld: &serde_json::Value,
    shape: FrontmatterShape,
) -> Result<(Frontmatter, String), FrontmatterError> {
    let object = jsonld
        .as_object()
        .ok_or(FrontmatterError::JsonNotAnObject)?;
    let mut frontmatter = Mapping::new();

    if shape == FrontmatterShape::V1Canonical {
        if let Some(id_value) = object.get("@id") {
            let id_str = id_value
                .as_str()
                .ok_or_else(|| FrontmatterError::FieldNotAString {
                    field: "@id".to_string(),
                })?;
            let id = id_str.strip_prefix("urn:mif:").unwrap_or(id_str);
            frontmatter.insert("id".into(), id.into());
        }
        if let Some(concept_type) = object.get("conceptType") {
            frontmatter.insert(
                "type".into(),
                json_value_to_yaml("conceptType", concept_type)?,
            );
        }
    }

    // `description` is only a derived mirror of `summary` when `summary`
    // itself is present in `jsonld` (see JSONLD_SPECIAL_KEYS's doc comment);
    // otherwise it is a genuine pass-through key that must round-trip.
    let description_is_derived =
        shape == FrontmatterShape::V1Canonical && object.contains_key("summary");
    for (key, value) in object {
        let is_excluded = match shape {
            FrontmatterShape::V1Canonical => {
                JSONLD_SPECIAL_KEYS.contains(&key.as_str())
                    || (key == "description" && description_is_derived)
            },
            FrontmatterShape::PreProjected => PRE_PROJECTED_EXCLUDED_KEYS.contains(&key.as_str()),
        };
        if is_excluded {
            continue;
        }
        frontmatter.insert(key.as_str().into(), json_value_to_yaml(key, value)?);
    }

    let body = object
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Ok((frontmatter, body))
}

/// Verifies that `md_text` survives a `markdown -> json-ld -> markdown`
/// round trip losslessly, mirroring `mif_convert.py`'s `roundtrip_file`.
///
/// "Losslessly" means the recovered markdown matches the *canonical*
/// serialization of the original frontmatter + body (via
/// [`serialize_markdown`]), not `md_text`'s literal bytes — the same
/// comparison [`FrontmatterError::RoundTripDrift`]'s `expected`/`recovered`
/// fields document. The JSON-LD projection is serialized to a JSON string
/// and re-parsed before recovering markdown from it, to simulate an
/// on-disk projection round trip rather than comparing in-memory values
/// directly.
///
/// # Errors
///
/// Returns any error from [`parse_markdown`], [`md_to_jsonld`],
/// [`jsonld_to_md`], or [`serialize_markdown`] along the way, or
/// [`FrontmatterError::RoundTripDrift`] if the recovered markdown differs
/// from the canonical serialization of the original.
///
/// # Examples
///
/// ```
/// let md = "---\nid: x\ntype: semantic\ncreated: 2026-01-01T00:00:00Z\n---\n\nBody.\n";
/// mif_frontmatter::roundtrip_lossless(md).unwrap();
/// ```
pub fn roundtrip_lossless(md_text: &str) -> Result<(), FrontmatterError> {
    let (frontmatter, body) = parse_markdown(md_text)?;
    let shape = detect_shape(&frontmatter);
    let jsonld = md_to_jsonld(&frontmatter, &body)?;

    let jsonld_text = serde_json::to_string(&jsonld)
        .map_err(|source| FrontmatterError::JsonRoundTrip { source })?;
    let jsonld: serde_json::Value = serde_json::from_str(&jsonld_text)
        .map_err(|source| FrontmatterError::JsonRoundTrip { source })?;

    let (recovered_frontmatter, recovered_body) = jsonld_to_md(&jsonld, shape)?;
    let recovered = serialize_markdown(&recovered_frontmatter, &recovered_body)?;
    let expected = serialize_markdown(&frontmatter, &body)?;

    if recovered != expected {
        return Err(FrontmatterError::RoundTripDrift {
            expected,
            recovered,
        });
    }
    Ok(())
}

/// Converts one YAML value to JSON, wrapping conversion failure with the
/// offending field's name for diagnostics.
fn yaml_value_to_json(field: &str, value: &Value) -> Result<serde_json::Value, FrontmatterError> {
    serde_json::to_value(value).map_err(|source| FrontmatterError::JsonConversion {
        field: field.to_string(),
        source,
    })
}

/// Converts one JSON value to YAML, wrapping conversion failure with the
/// offending field's name for diagnostics.
fn json_value_to_yaml(field: &str, value: &serde_json::Value) -> Result<Value, FrontmatterError> {
    serde_norway::to_value(value).map_err(|source| FrontmatterError::YamlConversion {
        field: field.to_string(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use mif_problem::ToProblem;

    use super::{
        FrontmatterError, FrontmatterShape, jsonld_to_md, md_to_jsonld, parse_markdown,
        roundtrip_lossless, serialize_markdown,
    };

    const RATE_LIMIT_POLICY: &str = r"---
id: 7b3c1e90-5a2f-4c8d-9e10-2f6a4b8c1d3e
type: semantic
created: 2026-01-15T10:30:00Z
modified: 2026-01-20T09:00:00Z
namespace: _semantic/policies
title: API Rate Limit Policy
summary: Declarative limits for the public API gateway.
tags:
  - api
  - policy
  - gateway
relationships:
  - type: derived-from
    target: /episodic/incident-2026-01-rate-spike.md
    strength: 0.9
  - type: relates-to
    target: /procedural/rotate-api-keys.md
---

# API Rate Limit Policy

Declarative knowledge: the public API gateway enforces a sliding-window rate
limit of 600 requests per minute per API key, with a burst allowance of 100.

## Rationale

The limit was set after observing sustained abuse traffic. It balances
legitimate batch consumers against gateway saturation.

## Relationships

- derived-from [Rate Spike Incident](/episodic/incident-2026-01-rate-spike.md)
- relates-to [Rotate API Keys](/procedural/rotate-api-keys.md)
";

    #[test]
    fn round_trips_the_rate_limit_policy_fixture_losslessly() {
        roundtrip_lossless(RATE_LIMIT_POLICY).unwrap();
    }

    #[test]
    fn parse_markdown_rejects_missing_frontmatter() {
        let err = parse_markdown("# Just a heading\n\nNo frontmatter here.\n").unwrap_err();
        assert!(matches!(err, FrontmatterError::MissingFrontmatter));
    }

    #[test]
    fn parse_markdown_rejects_unclosed_frontmatter() {
        let err = parse_markdown("---\nid: x\n").unwrap_err();
        assert!(matches!(err, FrontmatterError::MissingFrontmatter));
    }

    #[test]
    fn serialize_markdown_reorders_extra_keys_after_canonical_order() {
        let (frontmatter, body) = parse_markdown(
            "---\nzeta_extra: last\nid: x\ntype: semantic\nalpha_extra: also-extra\n---\n\nBody.\n",
        )
        .unwrap();
        let text = serialize_markdown(&frontmatter, &body).unwrap();
        assert_eq!(
            text,
            "---\nid: x\ntype: semantic\nzeta_extra: last\nalpha_extra: also-extra\n---\n\nBody.\n"
        );
    }

    #[test]
    fn full_roundtrip_preserves_keys_outside_frontmatter_order() {
        // Deliberate improvement over mif_convert.py (see the module doc
        // comment "Known deviations"): a genuinely unknown key (not in
        // FRONTMATTER_ORDER, not part of the canonical MIF schema) must
        // survive the full markdown -> json-ld -> markdown pipeline, not
        // just serialize_markdown alone.
        let md = "---\nid: x\ntype: semantic\ncustom_unknown_field: hello\n---\n\nBody.\n";
        roundtrip_lossless(md).unwrap();

        let (frontmatter, body) = parse_markdown(md).unwrap();
        let jsonld = md_to_jsonld(&frontmatter, &body).unwrap();
        assert_eq!(jsonld["custom_unknown_field"], "hello");
        let (recovered, _) = jsonld_to_md(&jsonld, FrontmatterShape::V1Canonical).unwrap();
        assert_eq!(
            recovered
                .get("custom_unknown_field")
                .and_then(|v| v.as_str()),
            Some("hello")
        );
    }

    #[test]
    fn full_roundtrip_preserves_keys_within_frontmatter_order() {
        let md = "---\nid: x\ntype: semantic\naliases:\n  - old-name\n---\n\nBody.\n";
        roundtrip_lossless(md).unwrap();
    }

    #[test]
    fn timestamp_falls_back_from_modified_to_created() {
        let (frontmatter, body) =
            parse_markdown("---\nid: x\ncreated: 2026-01-01T00:00:00Z\n---\n\nBody.\n").unwrap();
        let jsonld = md_to_jsonld(&frontmatter, &body).unwrap();
        assert_eq!(jsonld["timestamp"], "2026-01-01T00:00:00Z");
    }

    #[test]
    fn timestamp_prefers_modified_over_created() {
        let (frontmatter, body) = parse_markdown(
            "---\nid: x\ncreated: 2026-01-01T00:00:00Z\nmodified: 2026-02-02T00:00:00Z\n---\n\nBody.\n",
        )
        .unwrap();
        let jsonld = md_to_jsonld(&frontmatter, &body).unwrap();
        assert_eq!(jsonld["timestamp"], "2026-02-02T00:00:00Z");
    }

    #[test]
    fn datetime_values_come_through_as_plain_strings() {
        // serde_norway has no implicit YAML timestamp resolution (unlike
        // PyYAML), so unquoted timestamps already deserialize as strings
        // with no extra stringification step required.
        let (frontmatter, _) =
            parse_markdown("---\nid: x\ncreated: 2026-01-15T10:30:00Z\n---\n\nBody.\n").unwrap();
        let created = frontmatter.get("created").unwrap();
        assert_eq!(created.as_str(), Some("2026-01-15T10:30:00Z"));
    }

    #[test]
    fn md_to_jsonld_rejects_non_string_id() {
        let (frontmatter, body) = parse_markdown("---\nid: 42\n---\n\nBody.\n").unwrap();
        let err = md_to_jsonld(&frontmatter, &body).unwrap_err();
        assert!(matches!(err, FrontmatterError::FieldNotAString { field } if field == "id"));
    }

    #[test]
    fn jsonld_to_md_strips_urn_prefix_from_id() {
        let jsonld = serde_json::json!({"@id": "urn:mif:abc-123", "content": ""});
        let (frontmatter, _) = jsonld_to_md(&jsonld, FrontmatterShape::V1Canonical).unwrap();
        assert_eq!(
            frontmatter.get("id").and_then(|v| v.as_str()),
            Some("abc-123")
        );
    }

    #[test]
    fn jsonld_to_md_leaves_id_unchanged_without_urn_prefix() {
        let jsonld = serde_json::json!({"@id": "not-a-urn", "content": ""});
        let (frontmatter, _) = jsonld_to_md(&jsonld, FrontmatterShape::V1Canonical).unwrap();
        assert_eq!(
            frontmatter.get("id").and_then(|v| v.as_str()),
            Some("not-a-urn")
        );
    }

    #[test]
    fn jsonld_to_md_rejects_non_object_input() {
        let err = jsonld_to_md(
            &serde_json::json!(["not", "an", "object"]),
            FrontmatterShape::V1Canonical,
        )
        .unwrap_err();
        assert!(matches!(err, FrontmatterError::JsonNotAnObject));
    }

    #[test]
    fn empty_frontmatter_block_parses_to_empty_mapping() {
        let (frontmatter, body) = parse_markdown("---\n\n---\n\nBody.\n").unwrap();
        assert!(frontmatter.is_empty());
        assert_eq!(body, "\nBody.\n");
    }

    #[test]
    fn missing_frontmatter_and_roundtrip_drift_map_to_distinct_problem_types() {
        let missing = parse_markdown("no frontmatter here")
            .unwrap_err()
            .to_problem();
        assert_eq!(
            missing.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/missing-frontmatter/v1"
        );
        assert_eq!(missing.status, 422);
        assert_eq!(missing.exit_code, Some(2));
        assert!(missing.suggested_fix.is_some());

        let drift = FrontmatterError::RoundTripDrift {
            expected: "a".to_string(),
            recovered: "b".to_string(),
        }
        .to_problem();
        assert_eq!(
            drift.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/roundtrip-drift/v1"
        );
        assert_eq!(drift.exit_code, Some(4));
        assert_ne!(missing.problem_type, drift.problem_type);
    }

    #[test]
    fn frontmatter_shape_try_from_accepts_the_two_known_names() {
        assert_eq!(
            FrontmatterShape::try_from("v1-canonical").unwrap(),
            FrontmatterShape::V1Canonical
        );
        assert_eq!(
            FrontmatterShape::try_from("pre-projected").unwrap(),
            FrontmatterShape::PreProjected
        );
    }

    #[test]
    fn frontmatter_shape_try_from_rejects_unknown_names() {
        let error = FrontmatterShape::try_from("PreProjected").unwrap_err();
        assert!(matches!(error, FrontmatterError::UnknownShape(_)));
        let problem = error.to_problem();
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/unknown-frontmatter-shape/v1"
        );
        assert_eq!(problem.status, 400);
        assert_eq!(problem.exit_code, Some(2));
    }

    /// A fixture populating every field `mif.schema.json`'s root object
    /// defines, plus one field it does not (`custom_extra_field`) — proving
    /// the full spec surface, not just a sample of it, survives the round
    /// trip losslessly.
    const FULL_SPEC_FIXTURE: &str = r"---
id: memory:full-spec-001
type: semantic
memoryType: semantic
created: 2026-01-15T10:30:00Z
modified: 2026-01-20T09:00:00Z
namespace: _semantic/full-spec-demo
title: Full MIF Spec Surface Fixture
summary: Exercises every root-level field mif.schema.json defines.
properties:
  domain: testing
compressedAt: 2026-02-01T00:00:00Z
tags:
  - full-spec
  - fixture
aliases:
  - old-full-spec-name
temporal:
  '@type': TemporalMetadata
  validFrom: '2026-01-15T00:00:00Z'
  validUntil: '2027-01-15T00:00:00Z'
  ttl: P6M
  recordedAt: '2026-01-15T10:30:00Z'
  decay:
    model: exponential
    halfLife: P3M
    currentStrength: 0.9
provenance:
  '@type': Provenance
  sourceType: agent_inferred
  agent: claude-5-opus
  agentVersion: '2026.07'
  confidence: 0.85
  trustLevel: high_confidence
embedding:
  '@type': EmbeddingReference
  model: all-MiniLM-L6-v2
  modelVersion: '1.0'
  dimensions: 384
  normalized: true
  vectorUri: file:///tmp/vector.bin
relationships:
  - type: relates-to
    target: /semantic/another-fixture.md
    strength: 0.7
citations:
  - '@type': Citation
    citationType: article
    citationRole: supports
    title: A supporting source
    url: https://example.com/source
entities:
  - '@type': EntityReference
    entity:
      '@id': urn:mif:entity:person:jane-smith
    entityType: Person
documents:
  - '@type': DocumentReference
    url: https://example.com/doc.pdf
    title: Source PDF
    documentType: pdf
    contentType: application/pdf
ontology:
  id: trend-analysis
  version: 1.0.0
entity:
  name: Full spec fixture entity
  entity_type: emerging-issue
custom_extra_field: a value mif.schema.json does not define
---

Body content exercising every root-level MIF field.
";

    #[test]
    fn full_spec_fixture_round_trips_losslessly() {
        roundtrip_lossless(FULL_SPEC_FIXTURE).unwrap();
    }

    #[test]
    fn full_spec_fixture_projects_every_field_into_jsonld() {
        let (frontmatter, body) = parse_markdown(FULL_SPEC_FIXTURE).unwrap();
        let jsonld = md_to_jsonld(&frontmatter, &body).unwrap();

        assert_eq!(jsonld["@id"], "urn:mif:memory:full-spec-001");
        assert_eq!(jsonld["conceptType"], "semantic");
        assert_eq!(jsonld["memoryType"], "semantic");
        assert_eq!(jsonld["namespace"], "_semantic/full-spec-demo");
        assert_eq!(jsonld["temporal"]["validFrom"], "2026-01-15T00:00:00Z");
        assert_eq!(jsonld["temporal"]["decay"]["model"], "exponential");
        assert_eq!(jsonld["provenance"]["sourceType"], "agent_inferred");
        assert_eq!(jsonld["provenance"]["agent"], "claude-5-opus");
        assert_eq!(jsonld["embedding"]["dimensions"], 384);
        assert_eq!(jsonld["embedding"]["normalized"], true);
        assert_eq!(jsonld["citations"][0]["citationType"], "article");
        assert_eq!(jsonld["documents"][0]["@type"], "DocumentReference");
        assert_eq!(jsonld["documents"][0]["documentType"], "pdf");
        assert_eq!(jsonld["ontology"]["id"], "trend-analysis");
        assert_eq!(jsonld["entity"]["entity_type"], "emerging-issue");
        assert_eq!(
            jsonld["custom_extra_field"],
            "a value mif.schema.json does not define"
        );

        // Validates against the canonical schema this crate is meant to
        // feed, not just this crate's own round-trip check.
        mif_schema::validate_document(&jsonld).unwrap();
    }

    #[test]
    fn every_provenance_source_type_round_trips_losslessly() {
        for source_type in [
            "user_explicit",
            "user_implicit",
            "agent_inferred",
            "external_import",
            "system_generated",
        ] {
            let md = format!(
                "---\nid: x\ntype: semantic\ncreated: 2026-01-01T00:00:00Z\nprovenance:\n  '@type': Provenance\n  sourceType: {source_type}\n---\n\nBody.\n"
            );
            let result = roundtrip_lossless(&md);
            assert!(
                result.is_ok(),
                "sourceType {source_type} failed to round-trip: {result:?}"
            );

            let (frontmatter, body) = parse_markdown(&md).unwrap();
            let jsonld = md_to_jsonld(&frontmatter, &body).unwrap();
            assert_eq!(jsonld["provenance"]["sourceType"], source_type);
            mif_schema::validate_document(&jsonld).unwrap();
        }
    }

    #[test]
    fn document_reference_identified_by_id_instead_of_url_round_trips() {
        // DocumentReference's schema requires either `url` or `id` (anyOf),
        // not necessarily both — exercise the `id`-only branch too.
        let md = "---\nid: x\ntype: semantic\ncreated: 2026-01-01T00:00:00Z\ndocuments:\n  - '@type': DocumentReference\n    id: urn:doc:internal-note-1\n    title: Internal note\n---\n\nBody.\n";
        roundtrip_lossless(md).unwrap();

        let (frontmatter, body) = parse_markdown(md).unwrap();
        let jsonld = md_to_jsonld(&frontmatter, &body).unwrap();
        assert_eq!(jsonld["documents"][0]["id"], "urn:doc:internal-note-1");
        mif_schema::validate_document(&jsonld).unwrap();
    }

    #[test]
    fn the_real_rht_report_with_context_shaped_frontmatter_round_trips() {
        // Regression fixture for the actual failure this redesign fixes:
        // research-harness-template's Level-3 report frontmatter already
        // embeds `@context`/`@type`/`@id`/`conceptType` directly, plus a
        // `slug`/`version` pair the v1.0 canonical shape doesn't define.
        // Before the generic-passthrough redesign, `roundtrip_lossless`
        // rejected this with RoundTripDrift because `slug`/`version` (and
        // the already-JSON-LD-shaped keys) fell outside the fixed
        // FRONTMATTER_ORDER passthrough list.
        let md = r#"---
slug: reports/example-topic/report-exec-summary
version: 1
'@context': https://mif-spec.dev/schema/context.jsonld
'@type': Concept
'@id': urn:mif:report:harness/example-topic:report-exec-summary
conceptType: semantic
namespace: harness/example-topic
title: 'Executive Summary: An Example Report'
created: "2026-06-28T14:09:45Z"
provenance:
  '@type': Provenance
  sourceType: system_generated
  confidence: 0.9
  trustLevel: moderate_confidence
---

This exec-summary synthesis covers example findings.
"#;
        roundtrip_lossless(md).unwrap();
    }

    #[test]
    fn pre_projected_shape_does_not_synthesize_timestamp_when_absent() {
        // Regression test: md_to_jsonld must not unconditionally overwrite
        // `timestamp`/`description` for PreProjected documents just because
        // it does for V1Canonical ones.
        let md = "---\n'@id': urn:mif:x\nconceptType: semantic\ncreated: 2026-01-01T00:00:00Z\n---\n\nBody.\n";
        let (frontmatter, body) = parse_markdown(md).unwrap();
        let jsonld = md_to_jsonld(&frontmatter, &body).unwrap();
        assert!(jsonld.get("timestamp").is_none());
        assert!(jsonld.get("description").is_none());
    }

    #[test]
    fn pre_projected_shape_preserves_a_literal_timestamp_field_losslessly() {
        // The exact collision review-bugscan flagged: a PreProjected
        // document with its own literal `timestamp` field (not a
        // created/modified mirror) must not have that value clobbered or
        // dropped on round trip.
        let md = "---\n'@id': urn:mif:x\nconceptType: semantic\ncreated: 2026-01-01T00:00:00Z\ntimestamp: a-custom-literal-value\ndescription: a custom literal description\n---\n\nBody.\n";
        roundtrip_lossless(md).unwrap();

        let (frontmatter, body) = parse_markdown(md).unwrap();
        let jsonld = md_to_jsonld(&frontmatter, &body).unwrap();
        assert_eq!(jsonld["timestamp"], "a-custom-literal-value");
        assert_eq!(jsonld["description"], "a custom literal description");
    }

    #[test]
    fn v1_canonical_shape_still_synthesizes_timestamp_and_description() {
        // Confirms the fix above didn't regress V1Canonical's existing,
        // intentional derived-mirror behavior.
        let md = "---\nid: x\ntype: semantic\ncreated: 2026-01-01T00:00:00Z\nsummary: A summary.\n---\n\nBody.\n";
        let (frontmatter, body) = parse_markdown(md).unwrap();
        let jsonld = md_to_jsonld(&frontmatter, &body).unwrap();
        assert_eq!(jsonld["timestamp"], "2026-01-01T00:00:00Z");
        assert_eq!(jsonld["description"], "A summary.");
    }

    #[test]
    fn v1_canonical_shape_round_trips_a_literal_description_field_without_summary() {
        // Regression test for the mif-rs#38 bug: a V1Canonical document
        // carrying its own literal `description` frontmatter key (no
        // `summary` field to derive it from) failed roundtrip_lossless
        // because jsonld_to_md unconditionally treated `description` as a
        // derived-only mirror and dropped it.
        let md = "---\nid: memory:drift-a\ntype: semantic\ncreated: 2026-07-05T00:00:00Z\ndescription: any description here\ntags:\n- x\n---\n\nBody A.\n";
        roundtrip_lossless(md).unwrap();

        let (frontmatter, body) = parse_markdown(md).unwrap();
        let jsonld = md_to_jsonld(&frontmatter, &body).unwrap();
        assert_eq!(jsonld["description"], "any description here");
        assert!(jsonld.get("summary").is_none());
    }

    #[test]
    fn parse_markdown_rejects_invalid_yaml_syntax_and_maps_to_a_problem() {
        let err = parse_markdown("---\n[1, 2\n---\n\nBody.\n").unwrap_err();
        assert!(matches!(err, FrontmatterError::Yaml { .. }));
        let problem = err.to_problem();
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/invalid-frontmatter-yaml/v1"
        );
        assert_eq!(problem.status, 422);
        assert!(problem.suggested_fix.is_some());
    }

    #[test]
    fn parse_markdown_rejects_sequence_frontmatter_as_not_a_mapping_and_maps_to_a_problem() {
        let err = parse_markdown("---\n- a\n- b\n---\n\nBody.\n").unwrap_err();
        assert!(matches!(err, FrontmatterError::NotAMapping));
        let problem = err.to_problem();
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/frontmatter-not-a-mapping/v1"
        );
        assert_eq!(problem.status, 422);
        assert!(problem.suggested_fix.is_some());
    }

    #[test]
    fn md_to_jsonld_omits_id_when_frontmatter_has_none() {
        // Exercises the `if let Some(id_value) = frontmatter.get("id")`
        // branch's "absent" path — every other test in this suite supplies
        // an `id`, so the no-id case was otherwise never taken.
        let (frontmatter, body) = parse_markdown("---\ntype: semantic\n---\n\nBody.\n").unwrap();
        let jsonld = md_to_jsonld(&frontmatter, &body).unwrap();
        assert!(jsonld.get("@id").is_none());
        assert_eq!(jsonld["conceptType"], "semantic");
    }

    #[test]
    fn jsonld_to_md_omits_id_when_jsonld_has_none() {
        // Mirror of the test above, on the jsonld_to_md side: the
        // "@id absent" path of its own `if let Some(id_value)` branch.
        let jsonld = serde_json::json!({"conceptType": "semantic", "content": "Body."});
        let (frontmatter, body) = jsonld_to_md(&jsonld, FrontmatterShape::V1Canonical).unwrap();
        assert!(frontmatter.get("id").is_none());
        assert_eq!(body, "Body.");
    }

    #[test]
    fn jsonld_to_md_rejects_non_string_id_and_maps_to_a_problem() {
        let jsonld = serde_json::json!({"@id": 42, "content": ""});
        let err = jsonld_to_md(&jsonld, FrontmatterShape::V1Canonical).unwrap_err();
        assert!(matches!(err, FrontmatterError::FieldNotAString { ref field } if field == "@id"));
        let problem = err.to_problem();
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/field-not-a-string/v1"
        );
        assert_eq!(problem.status, 422);
    }

    #[test]
    fn jsonld_to_md_rejects_non_object_input_and_maps_to_a_problem() {
        let err = jsonld_to_md(
            &serde_json::json!(["not", "an", "object"]),
            FrontmatterShape::V1Canonical,
        )
        .unwrap_err();
        assert!(matches!(err, FrontmatterError::JsonNotAnObject));
        let problem = err.to_problem();
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/jsonld-not-an-object/v1"
        );
        assert_eq!(problem.status, 422);
    }

    #[test]
    fn non_string_top_level_frontmatter_key_is_dropped_and_causes_roundtrip_drift() {
        // A bare numeric key (`123: ...`) resolves to a non-string YAML
        // scalar; md_to_jsonld's generic pass-through loop can only key a
        // JSON object by a Rust &str, so it silently `continue`s past any
        // frontmatter key that isn't a string (see the `key.as_str()` else
        // branch). That silent drop is exactly what turns a real
        // roundtrip_lossless call into genuine RoundTripDrift, not just a
        // hand-constructed FrontmatterError::RoundTripDrift value.
        let md = "---\nid: x\ntype: semantic\n123: orphaned-value\n---\n\nBody.\n";

        let (frontmatter, body) = parse_markdown(md).unwrap();
        let jsonld = md_to_jsonld(&frontmatter, &body).unwrap();
        assert!(jsonld.get("123").is_none());

        let err = roundtrip_lossless(md).unwrap_err();
        assert!(matches!(err, FrontmatterError::RoundTripDrift { .. }));
    }

    #[test]
    fn a_complex_yaml_mapping_key_nested_in_a_field_fails_json_conversion() {
        // Real, spec-legal YAML (the explicit `? ... : ...` complex-key
        // form) that JSON's data model cannot represent: a mapping keyed by
        // a sequence. Unlike a non-string *top-level* frontmatter key (which
        // md_to_jsonld silently skips, see the test above), this key is
        // buried inside a field's *value*, so yaml_value_to_json actually
        // attempts — and fails — to convert it, hitting its JsonConversion
        // error path for real rather than by hand-constructing the variant.
        let md = "---\nid: x\ntype: semantic\nproperties:\n  ? [1, 2]\n  : value\n---\n\nBody.\n";
        let (frontmatter, body) = parse_markdown(md).unwrap();
        let err = md_to_jsonld(&frontmatter, &body).unwrap_err();
        assert!(
            matches!(err, FrontmatterError::JsonConversion { ref field, .. } if field == "properties")
        );
        let problem = err.to_problem();
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/field-json-conversion-failure/v1"
        );
        assert_eq!(problem.status, 422);
    }

    #[test]
    fn yaml_serialize_yaml_conversion_and_json_roundtrip_variants_map_to_expected_problems() {
        // These three variants' own error-construction call sites
        // (`YamlSerialize`/`YamlConversion`'s serde_norway failure and
        // `JsonRoundTrip`'s serde_json failure) are not reachable through
        // this crate's public functions with a legitimately constructed
        // input — see the module-level note on json_value_to_yaml's mirror
        // branch. Constructed directly here (each `#[source]` obtained from
        // a real, independently forced parse failure) purely to prove the
        // `meta()`/`to_problem()` mapping for each variant, per the two
        // "internal bug" arms of `FrontmatterError::to_problem`.
        let forced_yaml_error =
            || serde_norway::from_str::<serde_norway::Value>("[1, 2").unwrap_err();
        let forced_json_error =
            || serde_json::from_str::<serde_json::Value>("not json").unwrap_err();

        let yaml_serialize = FrontmatterError::YamlSerialize {
            source: forced_yaml_error(),
        };
        let problem = yaml_serialize.to_problem();
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/yaml-serialization-failure/v1"
        );
        assert_eq!(problem.status, 500);
        assert_eq!(problem.exit_code, Some(1));
        assert!(problem.suggested_fix.is_some());
        assert!(!problem.code_actions.is_empty());

        let yaml_conversion = FrontmatterError::YamlConversion {
            field: "some_field".to_string(),
            source: forced_yaml_error(),
        };
        let problem = yaml_conversion.to_problem();
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/field-yaml-conversion-failure/v1"
        );
        assert_eq!(problem.status, 422);

        let json_roundtrip = FrontmatterError::JsonRoundTrip {
            source: forced_json_error(),
        };
        let problem = json_roundtrip.to_problem();
        assert_eq!(
            problem.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/json-roundtrip-failure/v1"
        );
        assert_eq!(problem.status, 500);
        assert_eq!(problem.exit_code, Some(1));
        assert!(problem.suggested_fix.is_some());
        assert!(!problem.code_actions.is_empty());
    }
}
