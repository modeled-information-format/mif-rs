//! Dual-consumer error output: an RFC 9457 Problem Details envelope, shared
//! across the MIF (Modeled Information Format) workspace's crates.
//!
//! A command-line tool or MCP server now answers to two audiences: the human
//! who reads the terminal, and the LLM agent that parses the bytes and
//! decides whether to retry, escalate, or abandon. The human is served by an
//! error enum's ordinary `Display` output (unchanged). The agent is served by
//! [`ProblemDetails`] — a serializable [RFC 9457] *Problem Details* envelope
//! carrying the five standard members plus the three agent extensions
//! (`retry_after`, `suggested_fix`, `code_actions`) and an [`Applicability`]
//! marker on every suggested fix and code action.
//!
//! This workspace deliberately has **no shared top-level error type** (see
//! this repo's `CLAUDE.md`, "Why `thiserror` for Errors") — `mif-schema`,
//! `mif-ontology`, `mif-frontmatter`, `mif-embed`, and `mif-store` each fail
//! in genuinely different ways and keep their own `thiserror` enum. This
//! crate does not change that: instead of one central `Error` enum with a
//! `meta()` match (the pattern this crate adapts from
//! `attested-delivery/rust-template`'s `crates/problem.rs`), each crate's own
//! error enum implements [`ToProblem`] directly, using [`ProblemMeta`] to
//! keep its own per-variant type-URI/status/exit-code bookkeeping in one
//! place.
//!
//! [RFC 9457]: https://www.rfc-editor.org/rfc/rfc9457

use serde::{Deserialize, Serialize};

/// Base URI under which this workspace's problem-type identifiers are
/// namespaced.
///
/// Every implementer's `type` URI is derived as
/// `{ERROR_TYPE_BASE_URI}/{slug}/{version}` (e.g.
/// `https://modeled-information-format.github.io/mif-rs/references/errors/invalid-input/v1`),
/// and is dereferenceable: `docs/references/errors/{slug}/{version}.md`
/// publishes a real reference page at that path via this repo's GitHub
/// Pages site. `mif-spec.dev` is reserved for the normative MIF
/// specification itself, not this implementation's own tooling/error
/// reference — hence the repo-scoped Pages URL rather than the spec
/// domain.
pub const ERROR_TYPE_BASE_URI: &str =
    "https://modeled-information-format.github.io/mif-rs/references/errors";

/// How confidently an agent may apply a [`SuggestedFix`] or [`CodeAction`].
///
/// Modeled on the rustc diagnostic `Applicability` enum. Without this marker
/// an agent may apply a plausible-looking but wrong edit, so every suggested
/// fix and code action carries one. `Unspecified` is the safe default and
/// must be treated as `MaybeIncorrect` (escalate to a human) by consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Applicability {
    /// The agent may apply the edit and retry without human confirmation.
    MachineApplicable,
    /// The agent must escalate to a human before applying.
    MaybeIncorrect,
    /// The fix contains slots the agent must fill; lower confidence.
    HasPlaceholders,
    /// Applicability is unknown; consumers treat this as [`Self::MaybeIncorrect`].
    #[default]
    Unspecified,
}

/// A recovery suggestion tagged with an [`Applicability`] marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SuggestedFix {
    /// Free-text description of the recovery action.
    pub description: String,
    /// How confidently the fix may be applied.
    pub applicability: Applicability,
}

impl SuggestedFix {
    /// Creates a suggested fix from a description and an applicability marker.
    ///
    /// # Arguments
    ///
    /// * `description` - What the consumer should do to recover.
    /// * `applicability` - How confidently the fix may be applied.
    ///
    /// # Returns
    ///
    /// A new [`SuggestedFix`].
    #[must_use]
    pub fn new(description: impl Into<String>, applicability: Applicability) -> Self {
        Self {
            description: description.into(),
            applicability,
        }
    }
}

/// A structured edit an agent can apply directly, modeled on the LSP
/// `CodeAction` interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CodeAction {
    /// Short, human-readable title for the action.
    pub title: String,
    /// The kind of action (e.g. `"quickfix"`), following LSP conventions.
    pub kind: String,
    /// How confidently the action may be applied.
    pub applicability: Applicability,
}

impl CodeAction {
    /// Creates a code action from a title, kind, and applicability marker.
    ///
    /// # Arguments
    ///
    /// * `title` - Short summary of the action.
    /// * `kind` - LSP-style action kind, e.g. `"quickfix"`.
    /// * `applicability` - How confidently the action may be applied.
    ///
    /// # Returns
    ///
    /// A new [`CodeAction`].
    #[must_use]
    pub fn new(
        title: impl Into<String>,
        kind: impl Into<String>,
        applicability: Applicability,
    ) -> Self {
        Self {
            title: title.into(),
            kind: kind.into(),
            applicability,
        }
    }
}

/// Classifies a `std::io::Error` for RFC 9457 rendering.
///
/// Every crate wrapping a bare I/O failure (reading an input file, an
/// ontology definition, a cached model file, ...) uses this to agree on how
/// a likely path mistake differs from a genuine I/O fault.
///
/// [`std::io::ErrorKind::NotFound`] and [`std::io::ErrorKind::PermissionDenied`]
/// are treated as probably-caller-input mistakes — a 4xx status
/// (404/403 respectively) with [`Applicability::MaybeIncorrect`] and a
/// "verify the path" suggested fix. Every other kind is treated as a
/// genuine I/O fault — status 500 with [`Applicability::Unspecified`] and a
/// fix that does not imply user error, since an agent branching on the
/// numeric `status` field must not misclassify a disk/permissions failure as
/// something the caller can simply correct and retry.
///
/// # Returns
///
/// The `(status, suggested_fix, code_action)` triple to attach to the
/// envelope in place of the caller's own static defaults.
#[must_use]
pub fn classify_io_error(error: &std::io::Error) -> (u16, SuggestedFix, CodeAction) {
    match error.kind() {
        std::io::ErrorKind::NotFound => (
            404,
            SuggestedFix::new(
                "Verify the path exists, then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Correct the file path",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        std::io::ErrorKind::PermissionDenied => (
            403,
            SuggestedFix::new(
                "Verify the path is readable (check file/directory permissions), then retry.",
                Applicability::MaybeIncorrect,
            ),
            CodeAction::new(
                "Correct the file permissions or path",
                "quickfix",
                Applicability::MaybeIncorrect,
            ),
        ),
        _ => (
            500,
            SuggestedFix::new(
                "This indicates an I/O problem, not a mistaken path. Check disk and \
                 permissions state and retry.",
                Applicability::Unspecified,
            ),
            CodeAction::new(
                "Retry the operation",
                "quickfix",
                Applicability::Unspecified,
            ),
        ),
    }
}

/// An [RFC 9457] *Problem Details* envelope for machine consumers.
///
/// Serializes under the `application/problem+json` media type. It carries
/// the five standard members (`type`, `title`, `status`, `detail`,
/// `instance`), the three agent extensions (`retry_after`, `suggested_fix`,
/// `code_actions`), and the optional `exit_code` extension. `retry_after`
/// serializes even when `None` (as JSON `null`) so an agent never has to
/// guess whether a class is transient.
///
/// Build one with [`ProblemDetails::new`] and the `with_*` methods, or
/// (preferred, for a crate's own error enum) with [`ProblemMeta::into_details`].
///
/// [RFC 9457]: https://www.rfc-editor.org/rfc/rfc9457
///
/// # Examples
///
/// ```rust
/// use mif_problem::{Applicability, ProblemDetails, SuggestedFix};
///
/// let problem = ProblemDetails::new(
///     "https://modeled-information-format.github.io/mif-rs/references/errors/invalid-input/v1",
///     "Invalid input",
///     400,
///     "the supplied file was not valid JSON",
///     "urn:mif-cli:invalid-input",
/// )
/// .with_exit_code(2)
/// .with_suggested_fix(SuggestedFix::new(
///     "Check the file is well-formed JSON and retry.",
///     Applicability::MaybeIncorrect,
/// ));
///
/// assert_eq!(problem.status, 400);
/// assert_eq!(problem.retry_after, None);
/// assert!(problem.to_json().contains("\"type\""));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProblemDetails {
    /// A URI reference identifying the problem type. Stable and versioned.
    #[serde(rename = "type")]
    pub problem_type: String,
    /// Short, human-readable summary of the problem type. Stable per `type`.
    pub title: String,
    /// Numeric status mapping to a status class (see also `exit_code`).
    pub status: u16,
    /// Human-readable explanation specific to this occurrence.
    pub detail: String,
    /// URI reference identifying this specific occurrence.
    pub instance: String,
    /// When the operation may safely be retried (delta-seconds). Explicitly
    /// `null` for non-transient errors so agents do not have to guess.
    pub retry_after: Option<u64>,
    /// A recovery suggestion, tagged with an applicability marker.
    pub suggested_fix: Option<SuggestedFix>,
    /// Structured edits the agent can apply directly.
    pub code_actions: Vec<CodeAction>,
    /// The process exit code emitted alongside the error, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<u8>,
}

impl ProblemDetails {
    /// Creates an envelope from the five RFC 9457 standard members.
    ///
    /// `retry_after`, `suggested_fix`, `code_actions`, and `exit_code` start
    /// empty; add them with the `with_*` methods.
    ///
    /// # Arguments
    ///
    /// * `problem_type` - Stable, versioned problem-type URI.
    /// * `title` - Short summary, stable per `problem_type`.
    /// * `status` - Numeric status class.
    /// * `detail` - This-occurrence explanation.
    /// * `instance` - URI identifying this occurrence.
    ///
    /// # Returns
    ///
    /// A new [`ProblemDetails`] with no extensions set.
    #[must_use]
    pub fn new(
        problem_type: impl Into<String>,
        title: impl Into<String>,
        status: u16,
        detail: impl Into<String>,
        instance: impl Into<String>,
    ) -> Self {
        Self {
            problem_type: problem_type.into(),
            title: title.into(),
            status,
            detail: detail.into(),
            instance: instance.into(),
            retry_after: None,
            suggested_fix: None,
            code_actions: Vec::new(),
            exit_code: None,
        }
    }

    /// Sets `retry_after` to `seconds`, marking the error as transient.
    #[must_use]
    pub const fn with_retry_after(mut self, seconds: u64) -> Self {
        self.retry_after = Some(seconds);
        self
    }

    /// Attaches a [`SuggestedFix`].
    #[must_use]
    pub fn with_suggested_fix(mut self, fix: SuggestedFix) -> Self {
        self.suggested_fix = Some(fix);
        self
    }

    /// Appends a [`CodeAction`] to `code_actions`.
    #[must_use]
    pub fn with_code_action(mut self, action: CodeAction) -> Self {
        self.code_actions.push(action);
        self
    }

    /// Sets the `exit_code` extension.
    #[must_use]
    pub const fn with_exit_code(mut self, code: u8) -> Self {
        self.exit_code = Some(code);
        self
    }

    /// Serializes the envelope as a compact `application/problem+json` string.
    ///
    /// # Returns
    ///
    /// The compact JSON representation. Returns `"{}"` only if serialization
    /// fails, which cannot happen for this all-owned, self-describing struct.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| String::from("{}"))
    }

    /// Serializes the envelope as pretty-printed `application/problem+json`.
    ///
    /// # Returns
    ///
    /// The indented JSON representation, suitable for human inspection.
    #[must_use]
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| String::from("{}"))
    }
}

/// Reusable per-variant problem-type metadata.
///
/// An implementing crate defines one [`ProblemMeta`] per error variant (slug,
/// version, title, status, exit code) and converts it to a full
/// [`ProblemDetails`] with [`ProblemMeta::into_details`], keeping the
/// URI/version/status/exit-code bookkeeping for that crate's errors in one
/// place — extending the enum means adding one match arm, not editing several
/// parallel constructions.
#[derive(Debug, Clone, Copy)]
pub struct ProblemMeta {
    /// Stable, URL-safe slug for the problem type.
    pub slug: &'static str,
    /// Version segment of the type URI (e.g. `"v1"`). Per-type, so one type
    /// can advance independently of the others.
    pub version: &'static str,
    /// Short, stable title for the problem type.
    pub title: &'static str,
    /// Numeric status class.
    pub status: u16,
    /// Process exit code emitted alongside the error.
    pub exit_code: u8,
}

impl ProblemMeta {
    /// The stable, version-embedded problem-type URI for this metadata.
    ///
    /// Derived as `{ERROR_TYPE_BASE_URI}/{slug}/{version}`. The version is
    /// the stability commitment: the meaning of a given URI never changes; a
    /// breaking change to a problem type ships a new version (e.g. `/v2`)
    /// rather than redefining the existing one.
    ///
    /// # Returns
    ///
    /// The fully-qualified type URI for this metadata.
    #[must_use]
    pub fn type_uri(&self) -> String {
        format!("{ERROR_TYPE_BASE_URI}/{}/{}", self.slug, self.version)
    }

    /// Builds a [`ProblemDetails`] from this metadata, an owning crate name,
    /// and an occurrence-specific `detail` message.
    ///
    /// # Arguments
    ///
    /// * `crate_name` - The implementing crate's own name, e.g.
    ///   `env!("CARGO_PKG_NAME")` evaluated at the call site (this macro must
    ///   be invoked in the calling crate, not here, to expand correctly).
    /// * `detail` - This-occurrence explanation, typically the error's own
    ///   `Display` string so the human and machine renderings never drift.
    ///
    /// # Returns
    ///
    /// A [`ProblemDetails`] with `exit_code` pre-populated from this
    /// metadata; attach a `suggested_fix`/`code_action` with the `with_*`
    /// methods as needed.
    #[must_use]
    pub fn into_details(self, crate_name: &str, detail: impl Into<String>) -> ProblemDetails {
        ProblemDetails::new(
            self.type_uri(),
            self.title,
            self.status,
            detail,
            format!("urn:{crate_name}:{}", self.slug),
        )
        .with_exit_code(self.exit_code)
    }
}

/// Implemented by each crate's own error enum to map it to a
/// [`ProblemDetails`] envelope.
///
/// Keeps error enums scoped to each crate's own failure modes (this
/// workspace has no shared top-level error type) while sharing one envelope
/// shape across the workspace. Requires [`std::fmt::Display`] (already
/// derived by `thiserror::Error` on every implementing enum) so the default
/// [`ToProblem::render`] can reuse it for pretty output.
///
/// # Examples
///
/// ```rust
/// use mif_problem::{Applicability, CodeAction, ProblemDetails, ProblemMeta, SuggestedFix, ToProblem};
///
/// #[derive(Debug, thiserror::Error)]
/// enum ExampleError {
///     #[error("input was empty")]
///     Empty,
/// }
///
/// impl ToProblem for ExampleError {
///     fn to_problem(&self) -> ProblemDetails {
///         let meta = ProblemMeta {
///             slug: "empty-input",
///             version: "v1",
///             title: "Empty input",
///             status: 400,
///             exit_code: 2,
///         };
///         meta.into_details(env!("CARGO_PKG_NAME"), self.to_string())
///             .with_suggested_fix(SuggestedFix::new(
///                 "Supply a non-empty input.",
///                 Applicability::MaybeIncorrect,
///             ))
///             .with_code_action(CodeAction::new(
///                 "Provide a value",
///                 "quickfix",
///                 Applicability::MaybeIncorrect,
///             ))
///     }
/// }
///
/// let err = ExampleError::Empty;
/// assert_eq!(err.to_problem().status, 400);
/// assert_eq!(err.render(mif_problem::OutputFormat::Pretty), "Error: input was empty");
/// ```
pub trait ToProblem: std::fmt::Display {
    /// Maps `self` to a fully-populated [`ProblemDetails`] envelope.
    fn to_problem(&self) -> ProblemDetails;

    /// Renders `self` for the given [`OutputFormat`].
    ///
    /// Pretty rendering is `Error: {self}`, matching this workspace's
    /// existing `mif-cli`/`mif-mcp` error text. JSON rendering is the
    /// compact RFC 9457 envelope from [`ToProblem::to_problem`].
    ///
    /// # Arguments
    ///
    /// * `format` - The format to render.
    ///
    /// # Returns
    ///
    /// The rendered error string (without a trailing newline).
    fn render(&self, format: OutputFormat) -> String {
        match format {
            OutputFormat::Pretty => format!("Error: {self}"),
            OutputFormat::Json => self.to_problem().to_json(),
        }
    }
}

/// The rendering format for an error reported to a consumer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutputFormat {
    /// The human-readable `Error: {display}` line.
    Pretty,
    /// The RFC 9457 `application/problem+json` envelope.
    Json,
}

impl OutputFormat {
    /// Selects the output format for a consumer.
    ///
    /// JSON when `--format=json` is given explicitly, or when no format is
    /// given and the error stream is not a terminal. Pretty when
    /// `--format=pretty` is given, or when no format is given and the error
    /// stream is a terminal. An unrecognized explicit value falls back to
    /// the TTY heuristic.
    ///
    /// # Arguments
    ///
    /// * `explicit` - The value of an explicit `--format` flag, if any.
    /// * `is_terminal` - Whether the error stream is a TTY.
    ///
    /// # Returns
    ///
    /// The selected [`OutputFormat`].
    #[must_use]
    pub fn select(explicit: Option<&str>, is_terminal: bool) -> Self {
        match explicit {
            Some("json") => Self::Json,
            Some("pretty") => Self::Pretty,
            _ if is_terminal => Self::Pretty,
            _ => Self::Json,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Applicability, CodeAction, OutputFormat, ProblemDetails, ProblemMeta, SuggestedFix,
        ToProblem, classify_io_error,
    };

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error("invalid input: {0}")]
        InvalidInput(String),
        #[error("operation failed")]
        OperationFailed,
    }

    impl TestError {
        const fn meta(&self) -> ProblemMeta {
            match self {
                Self::InvalidInput(_) => ProblemMeta {
                    slug: "invalid-input",
                    version: "v1",
                    title: "Invalid input",
                    status: 400,
                    exit_code: 2,
                },
                Self::OperationFailed => ProblemMeta {
                    slug: "operation-failed",
                    version: "v1",
                    title: "Operation failed",
                    status: 500,
                    exit_code: 1,
                },
            }
        }
    }

    impl ToProblem for TestError {
        fn to_problem(&self) -> ProblemDetails {
            self.meta()
                .into_details("mif-problem-tests", self.to_string())
        }
    }

    #[test]
    fn applicability_serializes_snake_case() {
        let json = serde_json::to_string(&Applicability::MachineApplicable).unwrap();
        assert_eq!(json, "\"machine_applicable\"");
        assert_eq!(Applicability::default(), Applicability::Unspecified);
    }

    #[test]
    fn builder_sets_every_extension() {
        let problem = ProblemDetails::new("t", "T", 429, "d", "urn:x")
            .with_retry_after(180)
            .with_suggested_fix(SuggestedFix::new("wait", Applicability::MachineApplicable))
            .with_code_action(CodeAction::new(
                "retry",
                "quickfix",
                Applicability::MachineApplicable,
            ))
            .with_exit_code(2);

        assert_eq!(problem.retry_after, Some(180));
        assert_eq!(problem.exit_code, Some(2));
        assert_eq!(problem.code_actions.len(), 1);
        assert_eq!(
            problem.suggested_fix.unwrap().applicability,
            Applicability::MachineApplicable
        );
    }

    #[test]
    fn distinct_variants_map_to_distinct_versioned_envelopes() {
        let invalid = TestError::InvalidInput("bad".to_string()).to_problem();
        let failed = TestError::OperationFailed.to_problem();

        assert_eq!(
            invalid.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/invalid-input/v1"
        );
        assert_eq!(invalid.status, 400);
        assert_eq!(invalid.detail, "invalid input: bad");
        assert_eq!(invalid.instance, "urn:mif-problem-tests:invalid-input");
        assert_eq!(invalid.retry_after, None);
        assert_eq!(invalid.exit_code, Some(2));

        assert_eq!(
            failed.problem_type,
            "https://modeled-information-format.github.io/mif-rs/references/errors/operation-failed/v1"
        );
        assert_eq!(failed.status, 500);
        assert_eq!(failed.exit_code, Some(1));
        assert_ne!(invalid.problem_type, failed.problem_type);
    }

    #[test]
    fn json_envelope_carries_all_required_members() {
        let json = TestError::OperationFailed.to_problem().to_json();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();

        for member in ["type", "title", "status", "detail", "instance"] {
            assert!(value.get(member).is_some(), "missing {member}");
        }
        assert!(value.get("retry_after").is_some());
        assert!(value["retry_after"].is_null());
        assert_eq!(value["exit_code"], 1);
    }

    #[test]
    fn pretty_render_is_error_prefixed_display() {
        let err = TestError::OperationFailed;
        assert_eq!(err.render(OutputFormat::Pretty), "Error: operation failed");
    }

    #[test]
    fn json_render_matches_envelope_json() {
        let err = TestError::OperationFailed;
        assert_eq!(err.render(OutputFormat::Json), err.to_problem().to_json());
    }

    #[test]
    fn format_selection_honors_flag_then_tty() {
        assert_eq!(OutputFormat::select(Some("json"), true), OutputFormat::Json);
        assert_eq!(
            OutputFormat::select(Some("pretty"), false),
            OutputFormat::Pretty
        );
        assert_eq!(OutputFormat::select(None, true), OutputFormat::Pretty);
        assert_eq!(OutputFormat::select(None, false), OutputFormat::Json);
        assert_eq!(
            OutputFormat::select(Some("xml"), true),
            OutputFormat::Pretty
        );
    }

    #[test]
    fn envelope_round_trips_through_json() {
        let problem = TestError::OperationFailed.to_problem();
        let json = problem.to_json();
        let back: ProblemDetails = serde_json::from_str(&json).unwrap();
        assert_eq!(problem, back);
    }

    #[test]
    fn pretty_json_is_indented() {
        let pretty = TestError::OperationFailed.to_problem().to_json_pretty();
        assert!(pretty.contains('\n'));
        assert!(pretty.contains("  \"type\""));
    }

    #[test]
    fn classify_io_error_treats_not_found_as_a_likely_path_mistake() {
        let error = std::io::Error::from(std::io::ErrorKind::NotFound);
        let (status, fix, action) = classify_io_error(&error);
        assert_eq!(status, 404);
        assert_eq!(fix.applicability, Applicability::MaybeIncorrect);
        assert_eq!(action.applicability, Applicability::MaybeIncorrect);
    }

    #[test]
    fn classify_io_error_treats_permission_denied_as_a_likely_path_mistake() {
        let error = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let (status, fix, action) = classify_io_error(&error);
        assert_eq!(status, 403);
        assert_eq!(fix.applicability, Applicability::MaybeIncorrect);
        assert_eq!(action.applicability, Applicability::MaybeIncorrect);
    }

    #[test]
    fn classify_io_error_keeps_a_genuine_io_fault_at_500_and_does_not_imply_user_error() {
        let error = std::io::Error::from(std::io::ErrorKind::Other);
        let (status, fix, action) = classify_io_error(&error);
        assert_eq!(status, 500);
        assert_eq!(fix.applicability, Applicability::Unspecified);
        assert_eq!(action.applicability, Applicability::Unspecified);
    }

    /// `type_uri()` is `{ERROR_TYPE_BASE_URI}/{slug}/{version}` with no crate
    /// name in it, so a `slug` reused across two crates' `ProblemMeta`
    /// literals collides into one shared, indistinguishable problem type.
    /// Walks every crate's `src/` tree (workspace root two levels up from
    /// this crate's own manifest dir) and asserts every `slug: "..."`
    /// literal is workspace-unique, except entries explicitly allow-listed
    /// as intentionally-shared dead code that `to_problem()` never reaches
    /// (delegating match arms whose real problem comes from an inner
    /// error's own `to_problem()` instead of this crate's `meta()`).
    fn collect_rs_files(
        dir: &std::path::Path,
        out: &mut Vec<std::path::PathBuf>,
    ) -> Result<(), String> {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("failed to read dir {}: {e}", dir.display())),
        };
        for entry in entries {
            let path = entry
                .map_err(|e| format!("failed to read entry in {}: {e}", dir.display()))?
                .path();
            if path.is_dir() {
                collect_rs_files(&path, out)?;
                continue;
            }
            if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
        Ok(())
    }

    /// Every `slug: "..."` literal in a source file's text, in appearance order.
    fn slugs_in_file(path: &std::path::Path) -> Result<Vec<String>, String> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        Ok(contents
            .lines()
            .filter_map(|line| {
                let rest = line.trim_start().strip_prefix("slug: \"")?;
                let end = rest.find('"')?;
                Some(rest[..end].to_string())
            })
            .collect())
    }

    /// `type_uri()` is `{ERROR_TYPE_BASE_URI}/{slug}/{version}` with no crate
    /// name in it, so a `slug` reused across two crates' `ProblemMeta`
    /// literals collides into one shared, indistinguishable problem type.
    /// Walks every crate's `src/` tree (workspace root two levels up from
    /// this crate's own manifest dir) and asserts every `slug: "..."`
    /// literal is workspace-unique, except entries explicitly allow-listed
    /// as intentionally-shared dead code that `to_problem()` never reaches
    /// (delegating match arms whose real problem comes from an inner
    /// error's own `to_problem()` instead of this crate's `meta()`).
    #[test]
    fn every_problem_meta_slug_is_workspace_unique() {
        const ALLOWED_SHARED: &[&str] = &["delegated"];

        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("mif-problem lives at <workspace_root>/crates/mif-problem")
            .to_path_buf();
        let crates_dir = workspace_root.join("crates");

        let mut files = Vec::new();
        for crate_dir in std::fs::read_dir(&crates_dir)
            .expect("workspace crates/ directory must exist")
            .filter_map(Result::ok)
        {
            collect_rs_files(&crate_dir.path().join("src"), &mut files)
                .expect("crate src tree walk failed");
        }

        let mut occurrences: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for file in &files {
            for slug in slugs_in_file(file).expect("failed to read a collected .rs file") {
                occurrences
                    .entry(slug)
                    .or_default()
                    .push(file.display().to_string());
            }
        }

        let collisions: Vec<(String, Vec<String>)> = occurrences
            .into_iter()
            .filter(|(slug, files)| files.len() > 1 && !ALLOWED_SHARED.contains(&slug.as_str()))
            .collect();
        assert!(
            collisions.is_empty(),
            "duplicate ProblemMeta slug(s) collide into the same type_uri: {collisions:#?}"
        );
    }

    /// The quoted value of `field: "..."` within a single `ProblemMeta { ... }`
    /// literal's body text.
    fn quoted_field(literal: &str, field: &str) -> Option<String> {
        let needle = format!("{field}: \"");
        let start = literal.find(&needle)? + needle.len();
        let rest = &literal[start..];
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    }

    /// Every `slug`/`version` pair from each `ProblemMeta { ... }` literal in
    /// a file, in appearance order. Field order within the literal doesn't
    /// matter -- each literal's body is isolated (up to its first `}`, and
    /// `ProblemMeta`'s fields are all flat scalars, so there is no nested
    /// `{`/`}` to confuse that boundary) and `slug`/`version` are located
    /// independently within it, rather than assuming `slug` always precedes
    /// `version` on the next line -- a struct literal's field order is not
    /// guaranteed, and a line-by-line lookahead would silently skip (not
    /// flag) any literal that declared them in the other order, defeating
    /// this test's whole purpose of catching missing doc pages.
    fn slug_version_pairs_in_file(path: &std::path::Path) -> Result<Vec<(String, String)>, String> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
        let pairs = contents
            .split("ProblemMeta {")
            .skip(1)
            .filter_map(|block| {
                let literal = &block[..block.find('}').unwrap_or(block.len())];
                let slug = quoted_field(literal, "slug")?;
                let version = quoted_field(literal, "version")?;
                Some((slug, version))
            })
            .collect();
        Ok(pairs)
    }

    /// `type_uri()`'s dereferenceable claim (this crate's own doc comment
    /// on `type_uri()`: "That URI is dereferenceable: it resolves to the
    /// reference page documenting that exact problem type") is only true if
    /// every emitted `{slug}/{version}` pair actually has a matching page at
    /// `docs/references/errors/{slug}/{version}.md`. This is exactly the
    /// class of drift issue #68 found: two binaries' `ProblemMeta` literals
    /// were split into per-binary prefixed slugs without the docs being
    /// updated to match, so the published reference page was never the one
    /// a real client's `type` field actually pointed at. `ALLOWED_SHARED`'s
    /// `"delegated"` entries are placeholder slugs on match arms whose real
    /// problem comes from an inner error's own `to_problem()` (see
    /// `every_problem_meta_slug_is_workspace_unique`'s doc comment) -- they
    /// never reach `type_uri()` at runtime and have no page of their own.
    ///
    /// Scoped to `DOCUMENTED_CRATES` -- the crates this workspace's
    /// `errors/index.md` actually has a section for today (`mif-schema`,
    /// `mif-ontology`, `mif-frontmatter`, `mif-embed`, `mif-store`,
    /// `mif-cli`, `mif-mcp`, `mif-rh`), matching this repo's own
    /// `CLAUDE.md` "Error Handling" list of `ToProblem` implementors the
    /// public docs cover. `ALLOWED_SHARED` also exempts `mif-rh`'s three
    /// delegating placeholder slugs (`delegated-ontology`,
    /// `delegated-frontmatter`, `delegated-embed` -- one per inner error
    /// type it wraps, distinct from `mif-cli`/`mif-mcp`'s single shared
    /// literal `"delegated"`) for the same reason: they never reach
    /// `type_uri()` at runtime and have no page of their own.
    #[test]
    fn every_problem_meta_slug_has_a_doc_page() {
        const ALLOWED_SHARED: &[&str] = &[
            "delegated",
            "delegated-ontology",
            "delegated-frontmatter",
            "delegated-embed",
        ];
        const DOCUMENTED_CRATES: &[&str] = &[
            "mif-schema",
            "mif-ontology",
            "mif-frontmatter",
            "mif-embed",
            "mif-store",
            "mif-cli",
            "mif-mcp",
            "mif-rh",
        ];

        let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("mif-problem lives at <workspace_root>/crates/mif-problem")
            .to_path_buf();
        let crates_dir = workspace_root.join("crates");
        let errors_dir = workspace_root.join("docs/references/errors");

        let mut files = Vec::new();
        for crate_name in DOCUMENTED_CRATES {
            collect_rs_files(&crates_dir.join(crate_name).join("src"), &mut files)
                .expect("crate src tree walk failed");
        }

        let missing: Vec<String> = files
            .iter()
            .flat_map(|file| {
                slug_version_pairs_in_file(file)
                    .expect("failed to read a collected .rs file")
                    .into_iter()
                    .map(move |(slug, version)| (file, slug, version))
            })
            .filter(|(_, slug, _)| !ALLOWED_SHARED.contains(&slug.as_str()))
            .filter_map(|(file, slug, version)| {
                let page = errors_dir.join(&slug).join(format!("{version}.md"));
                let entry = format!(
                    "{slug}/{version} (from {}) -- expected {}",
                    file.display(),
                    page.display()
                );
                if page.is_file() { None } else { Some(entry) }
            })
            .collect();

        assert!(
            missing.is_empty(),
            "ProblemMeta slug(s) with no matching docs/references/errors page: {missing:#?}"
        );
    }
}
