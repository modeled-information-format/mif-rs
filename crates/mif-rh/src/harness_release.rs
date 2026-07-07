//! Harness-native release/versioning tooling (rht Category B, Story #298).
//!
//! Ports rht's `scripts/goal-version.sh`, `scripts/bump-version.sh`, and
//! `scripts/check-version-bump.sh` (ADR-0010's change-driven versioning) to
//! the compiled engine.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::error::MifRhError;

/// Computes a goal's content-hash identity: `gv-<sha256(normalized
/// goal)[:12]>`.
///
/// "Normalized" is the goal JSON with the lineage fields (`version`,
/// `supersedes`, `revision`) removed and all keys sorted, compact. Removing
/// the lineage fields makes the hash a stable function of the goal's
/// *content* — minting a new version never perturbs the hash of the
/// content it describes.
#[must_use]
pub fn goal_version_id(goal: &serde_json::Value) -> String {
    let mut normalized = goal.clone();
    if let Some(object) = normalized.as_object_mut() {
        object.remove("version");
        object.remove("supersedes");
        object.remove("revision");
    }
    let compact = to_sorted_compact_json(&normalized);
    let mut hasher = Sha256::new();
    hasher.update(compact.as_bytes());
    let digest = hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        });
    format!("gv-{}", &digest[..12])
}

/// Serializes `value` to compact JSON with object keys sorted, matching
/// `jq -cS`'s canonical form (needed for [`goal_version_id`]'s hash to be
/// independent of source key order).
fn to_sorted_compact_json(value: &serde_json::Value) -> String {
    fn sort(value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(map) => {
                let mut sorted: std::collections::BTreeMap<String, serde_json::Value> =
                    std::collections::BTreeMap::new();
                for (key, entry) in map {
                    sorted.insert(key.clone(), sort(entry));
                }
                serde_json::Value::Object(sorted.into_iter().collect())
            },
            serde_json::Value::Array(items) => {
                serde_json::Value::Array(items.iter().map(sort).collect())
            },
            other => other.clone(),
        }
    }
    // Sorted, then serialized without pretty-printing; serde_json's default
    // object serialization preserves insertion order, and `sort` above
    // already inserted in sorted key order.
    serde_json::to_string(&sort(value)).unwrap_or_default()
}

/// One `--pack` target resolved to its concrete files.
#[derive(Debug, Clone)]
struct PackTarget {
    name: String,
    plugin_path: PathBuf,
    skill_path: PathBuf,
    doc_path: PathBuf,
}

/// Options for [`bump_version`].
pub struct BumpOptions<'a> {
    /// Repo root (`harness.config.json` and friends resolve against this).
    pub root: &'a Path,
    /// `"patch"`, `"minor"`, `"major"`, or an explicit `X.Y.Z`.
    pub spec: &'a str,
    /// Component names to also bump (each under `packs/<family>/<name>/`,
    /// excluding `packs/ontologies/*`, which versions independently).
    pub packs: &'a [String],
    /// CHANGELOG date for the new section. Defaults to today (UTC) if
    /// omitted.
    pub date: Option<&'a str>,
    /// Dry run: validate and report, write nothing.
    pub check: bool,
}

/// The outcome of a [`bump_version`] call.
#[derive(Debug, Clone)]
pub struct BumpReport {
    /// The version before the bump.
    pub old_version: String,
    /// The version after the bump.
    pub new_version: String,
    /// The CHANGELOG date used.
    pub date: String,
    /// Component names also bumped.
    pub packs: Vec<String>,
    /// Whether files were actually written (`false` for `--check`).
    pub applied: bool,
}

const RELEASE_POINTER_FILE: &str = "harness.config.json";
const MARKETPLACE_FILE: &str = ".claude-plugin/marketplace.json";
const CHANGELOG_FILE: &str = "CHANGELOG.md";
const PACK_DOC_DIR: &str = "docs/reference/packs";

/// Whether `changelog` already has a `## [<version>]` section header —
/// matching `grep -q "^## \[$NEW\]"`'s prefix semantics, since the real
/// header line also carries a trailing `- YYYY-MM-DD` date.
fn changelog_has_section(changelog: &str, version: &str) -> bool {
    let prefix = format!("## [{version}]");
    changelog.lines().any(|line| line.starts_with(&prefix))
}

fn is_semver(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

fn parse_semver(text: &str) -> Option<(u64, u64, u64)> {
    let parts: Vec<&str> = text.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ))
}

/// Whether `a` is a semver release strictly ahead of `b`.
fn semver_gt(a: &str, b: &str) -> Result<bool, MifRhError> {
    let (a, b) = (
        parse_semver(a).ok_or_else(|| MifRhError::VersionNotSemver {
            value: a.to_string(),
        })?,
        parse_semver(b).ok_or_else(|| MifRhError::VersionNotSemver {
            value: b.to_string(),
        })?,
    );
    Ok(a > b)
}

fn read_json(path: &Path) -> Result<serde_json::Value, MifRhError> {
    let contents = std::fs::read_to_string(path).map_err(|source| MifRhError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str(&contents).map_err(|source| MifRhError::Json {
        path: path.display().to_string(),
        source,
    })
}

fn write_json_pretty(path: &Path, value: &serde_json::Value) -> Result<(), MifRhError> {
    let text = serde_json::to_string_pretty(value).map_err(|source| MifRhError::JsonSerialize {
        path: path.display().to_string(),
        source,
    })?;
    std::fs::write(path, format!("{text}\n")).map_err(|source| MifRhError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn read_text(path: &Path) -> Result<String, MifRhError> {
    std::fs::read_to_string(path).map_err(|source| MifRhError::Io {
        path: path.display().to_string(),
        source,
    })
}

fn write_text(path: &Path, contents: &str) -> Result<(), MifRhError> {
    std::fs::write(path, contents).map_err(|source| MifRhError::Io {
        path: path.display().to_string(),
        source,
    })
}

/// Resolves one `--pack` component name to its plugin/skill/doc files under
/// `root`, excluding `packs/ontologies/*` (those version independently).
fn resolve_pack_target(root: &Path, name: &str) -> Result<PackTarget, MifRhError> {
    let packs_dir = root.join("packs");
    let mut hit: Option<(String, PathBuf)> = None;
    let families = std::fs::read_dir(&packs_dir).map_err(|source| MifRhError::Io {
        path: packs_dir.display().to_string(),
        source,
    })?;
    for family_entry in families.flatten() {
        let family_path = family_entry.path();
        if !family_path.is_dir()
            || family_path.file_name().and_then(|n| n.to_str()) == Some("ontologies")
        {
            continue;
        }
        let candidate = family_path.join(name);
        if candidate.is_dir() {
            let family = family_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if hit.is_some() {
                return Err(MifRhError::PackAmbiguous {
                    name: name.to_string(),
                });
            }
            hit = Some((family, candidate));
        }
    }
    let Some((family, dir)) = hit else {
        return Err(MifRhError::PackNotFound {
            name: name.to_string(),
        });
    };

    let plugin_path = dir.join(".claude-plugin/plugin.json");
    if !plugin_path.is_file() {
        return Err(MifRhError::PackFileMissing {
            name: name.to_string(),
            path: plugin_path.display().to_string(),
        });
    }
    let skill_path =
        find_skill_md(&dir.join("skills")).ok_or_else(|| MifRhError::PackFileMissing {
            name: name.to_string(),
            path: dir.join("skills/*/*/SKILL.md").display().to_string(),
        })?;
    let doc_path = root.join(PACK_DOC_DIR).join(format!("{family}.md"));
    if !doc_path.is_file() {
        return Err(MifRhError::PackFileMissing {
            name: name.to_string(),
            path: doc_path.display().to_string(),
        });
    }
    Ok(PackTarget {
        name: name.to_string(),
        plugin_path,
        skill_path,
        doc_path,
    })
}

/// Finds the first `<skills_dir>/*/SKILL.md` (one level of skill-name
/// nesting under `skills/`), matching `find "$dir/skills" -mindepth 2
/// -maxdepth 2 -name SKILL.md | head -1`.
fn find_skill_md(skills_dir: &Path) -> Option<PathBuf> {
    let mut skill_dirs: Vec<PathBuf> = std::fs::read_dir(skills_dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    skill_dirs.sort();
    skill_dirs
        .into_iter()
        .map(|dir| dir.join("SKILL.md"))
        .find(|path| path.is_file())
}

/// Rewrites the first `version: X` line in SKILL.md's YAML frontmatter to
/// `version: <new>`.
fn rewrite_skill_version(contents: &str, new_version: &str) -> Option<String> {
    let mut done = false;
    let mut out = String::with_capacity(contents.len());
    for line in contents.lines() {
        if !done && line.starts_with("version:") {
            out.push_str("version: ");
            out.push_str(new_version);
            done = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    done.then_some(out)
}

/// The first `version:` line's value in SKILL.md's frontmatter, if any.
fn skill_version(contents: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        line.strip_prefix("version:")
            .map(|rest| rest.trim().trim_matches(['"', '\'']).to_string())
    })
}

/// Rewrites the first `**Version:** X` row inside `## <comp>`/`### <comp>`'s
/// section (bounded by the next heading of any level) to `**Version:**
/// <new>`.
fn rewrite_doc_version(contents: &str, component: &str, new_version: &str) -> Option<String> {
    let heading2 = format!("## {component}");
    let heading3 = format!("### {component}");
    let mut in_section = false;
    let mut done = false;
    let mut out = String::with_capacity(contents.len());
    for line in contents.lines() {
        if line.starts_with('#') {
            in_section = line == heading2 || line == heading3;
        }
        if in_section && !done && line.starts_with("**Version:**") {
            let _ = write!(out, "**Version:** {new_version}");
            done = true;
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    done.then_some(out)
}

/// The `**Version:**` row's value inside `component`'s section, if any.
fn doc_version(contents: &str, component: &str) -> Option<String> {
    let heading2 = format!("## {component}");
    let heading3 = format!("### {component}");
    let mut in_section = false;
    for line in contents.lines() {
        if line.starts_with('#') {
            in_section = line == heading2 || line == heading3;
        }
        if in_section && let Some(rest) = line.strip_prefix("**Version:**") {
            return Some(
                rest.trim()
                    .split(|c: char| !(c.is_ascii_digit() || c == '.'))
                    .next()?
                    .to_string(),
            );
        }
    }
    None
}

/// Whether `component`'s section (bounded by the next heading) exists and
/// has a `**Version:**` row.
fn doc_has_version_row(contents: &str, component: &str) -> bool {
    doc_version(contents, component).is_some()
}

/// Whether `component`'s section heading (`## <component>` or `###
/// <component>`) exists at all, independent of whether it carries a
/// `**Version:**` row.
fn doc_has_section(contents: &str, component: &str) -> bool {
    let heading2 = format!("## {component}");
    let heading3 = format!("### {component}");
    contents
        .lines()
        .any(|line| line == heading2 || line == heading3)
}

/// Change-driven version bump (ADR-0010).
///
/// Moves the release pointer (`harness.config.json` `.version`), the
/// marketplace catalog (`.claude-plugin/marketplace.json`
/// `.metadata.version`), inserts a dated CHANGELOG section, and — for each
/// named `--pack` — moves that component's own `plugin.json` version,
/// `SKILL.md` frontmatter version, and family-doc `**Version:**` row.
/// Validates every mutation before writing any of them (transactional: a
/// malformed input fails with the tree untouched), and self-verifies every
/// write afterward.
///
/// # Errors
///
/// Returns [`MifRhError::VersionNotSemver`] if the current version, a
/// pack's version, or the requested spec is not well-formed semver,
/// [`MifRhError::VersionUnchanged`] if the new version equals the current
/// one, [`MifRhError::PackNotFound`]/[`MifRhError::PackAmbiguous`]/
/// [`MifRhError::PackFileMissing`] for an unresolvable `--pack` target,
/// [`MifRhError::ChangelogAnchorMissing`] if the CHANGELOG has neither an
/// `[Unreleased]` anchor nor an existing section for the new version,
/// [`MifRhError::PackAheadOfRelease`] if a pack's current version is
/// already ahead of the new release, and [`MifRhError::VerificationFailed`]
/// if a post-write self-check finds a file that did not update.
pub fn bump_version(opts: &BumpOptions<'_>) -> Result<BumpReport, MifRhError> {
    let cfg_path = opts.root.join(RELEASE_POINTER_FILE);
    let market_path = opts.root.join(MARKETPLACE_FILE);
    let changelog_path = opts.root.join(CHANGELOG_FILE);

    let cfg = read_json(&cfg_path)?;
    let old_version = cfg
        .get("version")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| MifRhError::VersionMissing {
            path: cfg_path.display().to_string(),
        })?;
    let new_version = resolve_new_version(opts.spec, &old_version)?;
    let date = opts.date.map_or_else(
        || chrono::Utc::now().format("%Y-%m-%d").to_string(),
        str::to_string,
    );
    let targets: Vec<PackTarget> = opts
        .packs
        .iter()
        .map(|name| resolve_pack_target(opts.root, name))
        .collect::<Result<_, _>>()?;

    let changelog = read_text(&changelog_path)?;
    let has_new_section =
        validate_bump_preconditions(&changelog, &changelog_path, &new_version, &targets)?;

    if opts.check {
        return Ok(BumpReport {
            old_version,
            new_version,
            date,
            packs: opts.packs.to_vec(),
            applied: false,
        });
    }

    apply_bump(&ApplyBumpInputs {
        cfg,
        cfg_path: &cfg_path,
        market_path: &market_path,
        changelog: &changelog,
        changelog_path: &changelog_path,
        new_version: &new_version,
        date: &date,
        has_new_section,
        targets: &targets,
    })?;
    verify_bump(
        &cfg_path,
        &market_path,
        &changelog_path,
        &new_version,
        &targets,
    )?;

    Ok(BumpReport {
        old_version,
        new_version,
        date,
        packs: opts.packs.to_vec(),
        applied: true,
    })
}

/// Resolves `spec` (`"major"`/`"minor"`/`"patch"` or an explicit `X.Y.Z`)
/// against `old_version` into the concrete new version.
fn resolve_new_version(spec: &str, old_version: &str) -> Result<String, MifRhError> {
    if !is_semver(old_version) {
        return Err(MifRhError::VersionNotSemver {
            value: old_version.to_string(),
        });
    }
    let new_version = match spec {
        "major" | "minor" | "patch" => {
            let (major, minor, patch) =
                parse_semver(old_version).ok_or_else(|| MifRhError::VersionNotSemver {
                    value: old_version.to_string(),
                })?;
            match spec {
                "major" => format!("{}.0.0", major + 1),
                "minor" => format!("{major}.{}.0", minor + 1),
                _ => format!("{major}.{minor}.{}", patch + 1),
            }
        },
        explicit => {
            if !is_semver(explicit) {
                return Err(MifRhError::VersionNotSemver {
                    value: explicit.to_string(),
                });
            }
            explicit.to_string()
        },
    };
    if new_version == old_version {
        return Err(MifRhError::VersionUnchanged { value: new_version });
    }
    Ok(new_version)
}

/// Validates every mutation `apply_bump` would make, before any of them are
/// written (transactional: a malformed input fails with the tree
/// untouched). Returns whether the CHANGELOG already has a section for
/// `new_version`.
fn validate_bump_preconditions(
    changelog: &str,
    changelog_path: &Path,
    new_version: &str,
    targets: &[PackTarget],
) -> Result<bool, MifRhError> {
    let has_new_section = changelog_has_section(changelog, new_version);
    let has_anchor = changelog.lines().any(|l| l == "## [Unreleased]");
    if !has_new_section && !has_anchor {
        return Err(MifRhError::ChangelogAnchorMissing {
            path: changelog_path.display().to_string(),
        });
    }
    for target in targets {
        let plugin = read_json(&target.plugin_path)?;
        let pack_version = plugin
            .get("version")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if !is_semver(pack_version) {
            return Err(MifRhError::PackVersionInvalid {
                name: target.name.clone(),
                path: target.plugin_path.display().to_string(),
                value: pack_version.to_string(),
            });
        }
        if semver_gt(pack_version, new_version)? {
            return Err(MifRhError::PackAheadOfRelease {
                name: target.name.clone(),
                pack_version: pack_version.to_string(),
                new_version: new_version.to_string(),
            });
        }
        let skill_contents = read_text(&target.skill_path)?;
        if skill_version(&skill_contents).is_none() {
            return Err(MifRhError::PackFileMissing {
                name: target.name.clone(),
                path: format!("{} (no version: frontmatter)", target.skill_path.display()),
            });
        }
        let doc_contents = read_text(&target.doc_path)?;
        if !doc_has_section(&doc_contents, &target.name) {
            return Err(MifRhError::PackFileMissing {
                name: target.name.clone(),
                path: format!(
                    "{} (no ## {} section)",
                    target.doc_path.display(),
                    target.name
                ),
            });
        }
        if !doc_has_version_row(&doc_contents, &target.name) {
            return Err(MifRhError::PackFileMissing {
                name: target.name.clone(),
                path: format!(
                    "{} (## {} section has no **Version:** row)",
                    target.doc_path.display(),
                    target.name
                ),
            });
        }
    }
    Ok(has_new_section)
}

/// Inputs for [`apply_bump`], bundled to stay under this workspace's
/// too-many-arguments threshold.
struct ApplyBumpInputs<'a> {
    cfg: serde_json::Value,
    cfg_path: &'a Path,
    market_path: &'a Path,
    changelog: &'a str,
    changelog_path: &'a Path,
    new_version: &'a str,
    date: &'a str,
    has_new_section: bool,
    targets: &'a [PackTarget],
}

/// Writes every mutation `bump_version` makes: the release pointer, the
/// marketplace catalog, the CHANGELOG insertion, and each pack's three
/// stamps. Called only after [`validate_bump_preconditions`] has already
/// confirmed every one of these writes is well-formed.
fn apply_bump(inputs: &ApplyBumpInputs<'_>) -> Result<(), MifRhError> {
    let mut cfg = inputs.cfg.clone();
    cfg["version"] = serde_json::Value::String(inputs.new_version.to_string());
    write_json_pretty(inputs.cfg_path, &cfg)?;

    let mut market = read_json(inputs.market_path)?;
    if let Some(metadata) = market.get_mut("metadata") {
        metadata["version"] = serde_json::Value::String(inputs.new_version.to_string());
    }
    write_json_pretty(inputs.market_path, &market)?;

    if !inputs.has_new_section {
        let mut inserted = String::with_capacity(inputs.changelog.len() + 64);
        let mut done = false;
        for line in inputs.changelog.lines() {
            inserted.push_str(line);
            inserted.push('\n');
            if !done && line == "## [Unreleased]" {
                inserted.push('\n');
                let _ = writeln!(inserted, "## [{}] - {}", inputs.new_version, inputs.date);
                done = true;
            }
        }
        write_text(inputs.changelog_path, &inserted)?;
    }

    for target in inputs.targets {
        let mut plugin = read_json(&target.plugin_path)?;
        plugin["version"] = serde_json::Value::String(inputs.new_version.to_string());
        write_json_pretty(&target.plugin_path, &plugin)?;

        let skill_contents = read_text(&target.skill_path)?;
        if let Some(rewritten) = rewrite_skill_version(&skill_contents, inputs.new_version) {
            write_text(&target.skill_path, &rewritten)?;
        }

        let doc_contents = read_text(&target.doc_path)?;
        if let Some(rewritten) =
            rewrite_doc_version(&doc_contents, &target.name, inputs.new_version)
        {
            write_text(&target.doc_path, &rewritten)?;
        }
    }
    Ok(())
}

/// Confirms every file `apply_bump` touched now reads `new_version`.
fn verify_bump(
    cfg_path: &Path,
    market_path: &Path,
    changelog_path: &Path,
    new_version: &str,
    targets: &[PackTarget],
) -> Result<(), MifRhError> {
    let cfg_after = read_json(cfg_path)?;
    if cfg_after.get("version").and_then(serde_json::Value::as_str) != Some(new_version) {
        return Err(MifRhError::VerificationFailed {
            path: cfg_path.display().to_string(),
        });
    }
    let market_after = read_json(market_path)?;
    if market_after
        .get("metadata")
        .and_then(|m| m.get("version"))
        .and_then(serde_json::Value::as_str)
        != Some(new_version)
    {
        return Err(MifRhError::VerificationFailed {
            path: market_path.display().to_string(),
        });
    }
    let changelog_after = read_text(changelog_path)?;
    if !changelog_has_section(&changelog_after, new_version) {
        return Err(MifRhError::VerificationFailed {
            path: changelog_path.display().to_string(),
        });
    }
    for target in targets {
        let plugin_after = read_json(&target.plugin_path)?;
        if plugin_after
            .get("version")
            .and_then(serde_json::Value::as_str)
            != Some(new_version)
        {
            return Err(MifRhError::VerificationFailed {
                path: target.plugin_path.display().to_string(),
            });
        }
        let skill_after = read_text(&target.skill_path)?;
        if skill_version(&skill_after).as_deref() != Some(new_version) {
            return Err(MifRhError::VerificationFailed {
                path: target.skill_path.display().to_string(),
            });
        }
        let doc_after = read_text(&target.doc_path)?;
        if doc_version(&doc_after, &target.name).as_deref() != Some(new_version) {
            return Err(MifRhError::VerificationFailed {
                path: target.doc_path.display().to_string(),
            });
        }
    }
    Ok(())
}

/// One version-bump-gate failure (ADR-0010's two independent rules).
#[derive(Debug, Clone)]
pub enum VersionGateFailure {
    /// Rule A.1: a changed pack did not move its `plugin.json` version.
    PackNotBumped {
        /// The pack directory.
        pack: String,
        /// Its unchanged version.
        version: String,
    },
    /// Rule A.2: a changed core skill did not move its `SKILL.md` version.
    SkillNotBumped {
        /// The skill directory.
        skill: String,
        /// Its unchanged version.
        version: String,
    },
    /// Rule B: the release pointer is not ahead of the last release tag.
    PointerNotAhead {
        /// The current release pointer.
        current: String,
        /// The last released tag (without the leading `v`).
        last_release: String,
    },
    /// Rule B: `harness.config.json` has no `.version` at HEAD.
    PointerMissing,
}

/// The outcome of a [`check_version_bump`] call.
#[derive(Debug, Clone, Default)]
pub struct VersionGateReport {
    /// Every rule violation found, empty if the gate passes.
    pub failures: Vec<VersionGateFailure>,
    /// The release pointer's value at HEAD, if readable.
    pub pointer_at_head: Option<String>,
}

impl VersionGateReport {
    /// Whether every rule held.
    #[must_use]
    pub const fn ok(&self) -> bool {
        self.failures.is_empty()
    }
}

fn run_git(root: &Path, args: &[&str]) -> Result<String, MifRhError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|source| MifRhError::Io {
            path: "git".to_string(),
            source,
        })?;
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn git_show_json_version(root: &Path, rev: &str, path: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["show", &format!("{rev}:{path}")])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value.get("version")?.as_str().map(str::to_string)
}

fn git_show_frontmatter_version(root: &Path, rev: &str, path: &str) -> Option<String> {
    let output = Command::new("git")
        .args(["show", &format!("{rev}:{path}")])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    skill_version(&text)
}

/// Enforces ADR-0010's change-driven versioning invariants against `base`.
///
/// Every changed pack/core-skill under `packs/` (excluding
/// `packs/ontologies/*`) or `.claude/skills/` must move its own version,
/// and `harness.config.json`'s release pointer must stay strictly ahead of
/// the last `v*` release tag.
///
/// # Errors
///
/// Returns [`MifRhError::Io`] if `git` cannot be invoked.
pub fn check_version_bump(root: &Path, base: &str) -> Result<VersionGateReport, MifRhError> {
    let merge_base = {
        let candidate = run_git(root, &["merge-base", base, "HEAD"])?;
        if candidate.is_empty() {
            base.to_string()
        } else {
            candidate
        }
    };
    let changed_output = run_git(
        root,
        &["diff", "--name-only", &format!("{merge_base}...HEAD")],
    )?;
    let changed: Vec<&str> = changed_output.lines().filter(|l| !l.is_empty()).collect();

    let mut failures = Vec::new();
    if !changed.is_empty() {
        failures.extend(check_changed_packs(root, &merge_base, &changed));
        failures.extend(check_changed_skills(root, &merge_base, &changed));
    }
    check_release_pointer(root, &mut failures);

    let pointer_at_head = read_json(&root.join(RELEASE_POINTER_FILE))
        .ok()
        .and_then(|v| {
            v.get("version")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });

    Ok(VersionGateReport {
        failures,
        pointer_at_head,
    })
}

/// Rule A.1: every changed pack (excluding `packs/ontologies/*`) must move
/// its own `plugin.json` `.version`.
fn check_changed_packs(root: &Path, merge_base: &str, changed: &[&str]) -> Vec<VersionGateFailure> {
    let changed_packs: BTreeSet<String> = changed
        .iter()
        .filter(|path| path.starts_with("packs/") && !path.starts_with("packs/ontologies/"))
        .filter_map(|path| {
            let parts: Vec<&str> = path.split('/').collect();
            // packs/<family>/<component>/... — need family AND component,
            // not just family.
            (parts.len() > 3).then(|| format!("{}/{}/{}", parts[0], parts[1], parts[2]))
        })
        .collect();
    let mut failures = Vec::new();
    for pack in changed_packs {
        let plugin_rel = format!("{pack}/.claude-plugin/plugin.json");
        if !root.join(&plugin_rel).is_file() {
            continue; // pack removed at HEAD — no requirement
        }
        let Some(base_version) = git_show_json_version(root, merge_base, &plugin_rel) else {
            continue; // new pack, absent at base
        };
        let head_version = read_json(&root.join(&plugin_rel))
            .ok()
            .and_then(|v| {
                v.get("version")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_default();
        if base_version == head_version {
            failures.push(VersionGateFailure::PackNotBumped {
                pack,
                version: head_version,
            });
        }
    }
    failures
}

/// Rule A.2: every changed core skill must move its own `SKILL.md`
/// frontmatter `version:`.
fn check_changed_skills(
    root: &Path,
    merge_base: &str,
    changed: &[&str],
) -> Vec<VersionGateFailure> {
    let changed_skills: BTreeSet<String> = changed
        .iter()
        .filter(|path| path.starts_with(".claude/skills/"))
        .filter_map(|path| {
            let parts: Vec<&str> = path.split('/').collect();
            (parts.len() > 3).then(|| parts[..3].join("/"))
        })
        .collect();
    let mut failures = Vec::new();
    for skill in changed_skills {
        let skill_rel = format!("{skill}/SKILL.md");
        if !root.join(&skill_rel).is_file() {
            continue;
        }
        let Some(base_version) = git_show_frontmatter_version(root, merge_base, &skill_rel) else {
            continue;
        };
        let head_version = read_text(&root.join(&skill_rel))
            .ok()
            .and_then(|c| skill_version(&c))
            .unwrap_or_default();
        if base_version == head_version {
            failures.push(VersionGateFailure::SkillNotBumped {
                skill,
                version: head_version,
            });
        }
    }
    failures
}

/// Rule B: the release pointer must stay strictly ahead of the last `v*`
/// release tag.
fn check_release_pointer(root: &Path, failures: &mut Vec<VersionGateFailure>) {
    let tags_output = run_git(root, &["tag", "--list", "v*"]);
    let last_release = tags_output.ok().and_then(|tags| {
        tags.lines()
            .filter_map(|tag| tag.strip_prefix('v'))
            .filter_map(parse_semver)
            .max()
    });
    let Some((lm, ln, lp)) = last_release else {
        return;
    };
    let head_version = read_json(&root.join(RELEASE_POINTER_FILE))
        .ok()
        .and_then(|v| {
            v.get("version")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        });
    match head_version {
        None => failures.push(VersionGateFailure::PointerMissing),
        Some(head_version) => {
            let head_parsed = parse_semver(&head_version).unwrap_or((0, 0, 0));
            if head_parsed <= (lm, ln, lp) {
                failures.push(VersionGateFailure::PointerNotAhead {
                    current: head_version,
                    last_release: format!("{lm}.{ln}.{lp}"),
                });
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::goal_version_id;

    #[test]
    fn goal_version_id_ignores_lineage_fields() {
        let a = serde_json::json!({"question": "why", "version": 1});
        let b = serde_json::json!({"question": "why", "version": 2, "supersedes": "gv-abc"});
        assert_eq!(goal_version_id(&a), goal_version_id(&b));
    }

    #[test]
    fn goal_version_id_is_independent_of_key_order() {
        let a = serde_json::json!({"question": "why", "scope": "x"});
        let b = serde_json::json!({"scope": "x", "question": "why"});
        assert_eq!(goal_version_id(&a), goal_version_id(&b));
    }

    #[test]
    fn goal_version_id_changes_with_content() {
        let a = serde_json::json!({"question": "why"});
        let b = serde_json::json!({"question": "why not"});
        assert_ne!(goal_version_id(&a), goal_version_id(&b));
    }

    #[test]
    fn goal_version_id_has_the_expected_shape() {
        let id = goal_version_id(&serde_json::json!({"question": "why"}));
        assert!(id.starts_with("gv-"));
        assert_eq!(id.len(), "gv-".len() + 12);
    }

    use super::{BumpOptions, PACK_DOC_DIR, bump_version};
    use std::fmt::Write as _;
    use std::fs;

    fn write_base_fixture(root: &std::path::Path) {
        fs::write(
            root.join("harness.config.json"),
            r#"{"version": "0.4.0", "topics": []}"#,
        )
        .unwrap();
        fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        fs::write(
            root.join(".claude-plugin/marketplace.json"),
            r#"{"name": "research-harness", "metadata": {"version": "0.4.0"}}"#,
        )
        .unwrap();
        fs::write(
            root.join("CHANGELOG.md"),
            "# Changelog\n\n## [Unreleased]\n\n## [0.4.0] - 2026-01-01\n\nInitial.\n",
        )
        .unwrap();
    }

    fn write_pack_fixture(root: &std::path::Path, family: &str, name: &str, version: &str) {
        let dir = root.join("packs").join(family).join(name);
        fs::create_dir_all(dir.join(".claude-plugin")).unwrap();
        fs::write(
            dir.join(".claude-plugin/plugin.json"),
            format!(r#"{{"name": "{name}", "version": "{version}"}}"#),
        )
        .unwrap();
        fs::create_dir_all(dir.join("skills").join(format!("{name}-skill"))).unwrap();
        fs::write(
            dir.join("skills")
                .join(format!("{name}-skill"))
                .join("SKILL.md"),
            format!("---\nversion: {version}\n---\n\n# {name}\n"),
        )
        .unwrap();
        fs::create_dir_all(root.join(PACK_DOC_DIR)).unwrap();
        let doc_path = root.join(PACK_DOC_DIR).join(format!("{family}.md"));
        let mut existing = fs::read_to_string(&doc_path).unwrap_or_default();
        let _ = write!(existing, "\n## {name}\n\n**Version:** {version}\n");
        fs::write(&doc_path, existing).unwrap();
    }

    #[test]
    fn bump_version_patch_moves_the_release_pointer_and_inserts_changelog() {
        let dir = tempfile::tempdir().unwrap();
        write_base_fixture(dir.path());

        let report = bump_version(&BumpOptions {
            root: dir.path(),
            spec: "patch",
            packs: &[],
            date: Some("2026-02-01"),
            check: false,
        })
        .unwrap();

        assert_eq!(report.old_version, "0.4.0");
        assert_eq!(report.new_version, "0.4.1");
        assert!(report.applied);

        let cfg: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.path().join("harness.config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(cfg["version"], "0.4.1");
        let market: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.path().join(".claude-plugin/marketplace.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(market["metadata"]["version"], "0.4.1");
        let changelog = fs::read_to_string(dir.path().join("CHANGELOG.md")).unwrap();
        assert!(changelog.contains("## [0.4.1] - 2026-02-01"));
    }

    #[test]
    fn bump_version_check_mode_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        write_base_fixture(dir.path());

        let report = bump_version(&BumpOptions {
            root: dir.path(),
            spec: "minor",
            packs: &[],
            date: Some("2026-02-01"),
            check: true,
        })
        .unwrap();

        assert!(!report.applied);
        assert_eq!(report.new_version, "0.5.0");
        let cfg: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.path().join("harness.config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(cfg["version"], "0.4.0", "check mode must not write");
    }

    #[test]
    fn bump_version_rejects_an_unchanged_version() {
        let dir = tempfile::tempdir().unwrap();
        write_base_fixture(dir.path());

        let error = bump_version(&BumpOptions {
            root: dir.path(),
            spec: "0.4.0",
            packs: &[],
            date: Some("2026-02-01"),
            check: false,
        })
        .unwrap_err();
        assert!(matches!(error, super::MifRhError::VersionUnchanged { .. }));
    }

    #[test]
    fn bump_version_bumps_a_named_pack_across_all_three_files() {
        let dir = tempfile::tempdir().unwrap();
        write_base_fixture(dir.path());
        write_pack_fixture(dir.path(), "channels", "pdf", "0.4.0");

        let report = bump_version(&BumpOptions {
            root: dir.path(),
            spec: "patch",
            packs: &["pdf".to_string()],
            date: Some("2026-02-01"),
            check: false,
        })
        .unwrap();
        assert_eq!(report.packs, ["pdf"]);

        let plugin: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(
                dir.path()
                    .join("packs/channels/pdf/.claude-plugin/plugin.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(plugin["version"], "0.4.1");
        let skill = fs::read_to_string(
            dir.path()
                .join("packs/channels/pdf/skills/pdf-skill/SKILL.md"),
        )
        .unwrap();
        assert!(skill.contains("version: 0.4.1"));
        let doc = fs::read_to_string(dir.path().join("docs/reference/packs/channels.md")).unwrap();
        assert!(doc.contains("**Version:** 0.4.1"));
    }

    #[test]
    fn bump_version_rejects_a_pack_already_ahead_of_the_new_release() {
        let dir = tempfile::tempdir().unwrap();
        write_base_fixture(dir.path());
        write_pack_fixture(dir.path(), "channels", "pdf", "9.0.0");

        let error = bump_version(&BumpOptions {
            root: dir.path(),
            spec: "patch",
            packs: &["pdf".to_string()],
            date: Some("2026-02-01"),
            check: false,
        })
        .unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::PackAheadOfRelease { .. }
        ));
        // Nothing should have been written: pre-flight runs before any apply.
        let cfg: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.path().join("harness.config.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(cfg["version"], "0.4.0");
    }

    #[test]
    fn bump_version_rejects_a_missing_changelog_anchor() {
        let dir = tempfile::tempdir().unwrap();
        write_base_fixture(dir.path());
        fs::write(
            dir.path().join("CHANGELOG.md"),
            "# Changelog\n\nNo anchor here.\n",
        )
        .unwrap();

        let error = bump_version(&BumpOptions {
            root: dir.path(),
            spec: "patch",
            packs: &[],
            date: Some("2026-02-01"),
            check: false,
        })
        .unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::ChangelogAnchorMissing { .. }
        ));
    }

    use super::{VersionGateFailure, check_version_bump};
    use std::process::Command;

    fn git(root: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?} failed");
    }

    fn init_repo(root: &std::path::Path) {
        git(root, &["init", "-q", "-b", "main"]);
        git(root, &["config", "user.email", "test@example.com"]);
        git(root, &["config", "user.name", "Test"]);
    }

    #[test]
    fn check_version_bump_passes_when_a_changed_pack_moved_its_own_version() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        write_base_fixture(dir.path());
        write_pack_fixture(dir.path(), "channels", "pdf", "0.4.0");
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-q", "-m", "base"]);
        git(dir.path(), &["tag", "v0.4.0"]);
        git(dir.path(), &["checkout", "-q", "-b", "feature"]);

        // Change the pack's content and bump its own version.
        std::fs::write(
            dir.path()
                .join("packs/channels/pdf/.claude-plugin/plugin.json"),
            r#"{"name": "pdf", "version": "0.4.1"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("harness.config.json"),
            r#"{"version": "0.4.1", "topics": []}"#,
        )
        .unwrap();
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-q", "-m", "bump pdf"]);

        let report = check_version_bump(dir.path(), "main").unwrap();
        assert!(
            report.ok(),
            "expected no failures, got {:?}",
            report.failures
        );
    }

    #[test]
    fn check_version_bump_fails_when_a_changed_pack_did_not_move_its_version() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        write_base_fixture(dir.path());
        write_pack_fixture(dir.path(), "channels", "pdf", "0.4.0");
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-q", "-m", "base"]);
        git(dir.path(), &["checkout", "-q", "-b", "feature"]);

        // Touch the pack's content WITHOUT bumping its version.
        std::fs::write(
            dir.path()
                .join("packs/channels/pdf/skills/pdf-skill/SKILL.md"),
            "---\nversion: 0.4.0\n---\n\n# pdf (edited)\n",
        )
        .unwrap();
        git(dir.path(), &["add", "-A"]);
        git(
            dir.path(),
            &["commit", "-q", "-m", "edit pdf, forgot to bump"],
        );

        let report = check_version_bump(dir.path(), "main").unwrap();
        assert!(!report.ok());
        assert!(
            report
                .failures
                .iter()
                .any(|f| matches!(f, VersionGateFailure::PackNotBumped { pack, .. } if pack == "packs/channels/pdf"))
        );
    }

    #[test]
    fn check_version_bump_fails_when_the_release_pointer_is_not_ahead_of_the_last_tag() {
        let dir = tempfile::tempdir().unwrap();
        init_repo(dir.path());
        write_base_fixture(dir.path());
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-q", "-m", "base"]);
        git(dir.path(), &["tag", "v0.4.0"]);
        git(dir.path(), &["checkout", "-q", "-b", "feature"]);
        std::fs::write(dir.path().join("README.md"), "unrelated change\n").unwrap();
        git(dir.path(), &["add", "-A"]);
        git(dir.path(), &["commit", "-q", "-m", "unrelated"]);

        let report = check_version_bump(dir.path(), "main").unwrap();
        assert!(!report.ok());
        assert!(
            report
                .failures
                .iter()
                .any(|f| matches!(f, VersionGateFailure::PointerNotAhead { .. }))
        );
    }
}
