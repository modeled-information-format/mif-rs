//! On-demand ontology vendoring (rht ADR-0012).
//!
//! Fetches domain ontology packs from the canonical registry, sha256-verifies
//! and pins them, catalogs them, discovers new registry entries, and checks
//! for local drift.
//!
//! Ports rht's `scripts/fetch-ontology.sh`, the ontology-catalog section of
//! `scripts/sync-packs.sh`, `scripts/check-ontology-lock.sh`, and
//! `scripts/sync-registry-ontologies.sh` to the compiled engine (Epic
//! research-harness-template#276, Story #277).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::MifRhError;
use crate::ontology_pack::parse_pack;

/// The canonical ontology registry's default base URL.
pub const DEFAULT_REGISTRY_SOURCE: &str = "https://mif-spec.dev/ontologies";

/// Ontology ids that are always committed base layers, never vendored, and
/// implicitly core when cataloged (mirrors `sync-packs.sh`'s `CORE_IDS`).
const CORE_IDS: [&str; 3] = ["mif-base", "mif-generic", "shared-traits"];

/// One registry index entry: a domain ontology's pinned version, sha256,
/// vendored file name, and `extends` ancestry.
#[derive(Debug, Clone, Deserialize)]
pub struct IndexEntry {
    /// The ontology's canonical version.
    pub version: String,
    /// The expected sha256 of the vendored file.
    pub sha256: String,
    /// The bare filename to fetch (must not contain a path separator).
    pub file: String,
    /// Ontology ids this ontology directly extends.
    #[serde(default)]
    pub extends: Vec<String>,
}

/// The canonical registry's `index.json`: every known domain ontology,
/// keyed by id.
#[derive(Debug, Clone, Deserialize)]
pub struct RegistryIndex {
    /// The indexed ontologies.
    pub ontologies: BTreeMap<String, IndexEntry>,
}

/// One pinned lock entry: a vendored ontology's version and verified
/// sha256.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockEntry {
    /// The vendored version.
    pub version: String,
    /// The verified sha256 of the vendored file.
    pub sha256: String,
}

/// `ontologies.lock.json`: the registry source, its trust-pinned index
/// sha256 (trust-on-first-use), and every vendored ontology's pin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockFile {
    /// The lock file's schema tag.
    #[serde(default = "lock_schema")]
    pub schema: String,
    /// The registry source this lock was populated from.
    #[serde(default)]
    pub source: String,
    /// The pinned registry index sha256, once one has been fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_sha256: Option<String>,
    /// Every vendored ontology's pin, keyed by id.
    #[serde(default)]
    pub ontologies: BTreeMap<String, LockEntry>,
}

fn lock_schema() -> String {
    "mif-ontology-lock/v1".to_string()
}

impl Default for LockFile {
    fn default() -> Self {
        Self {
            schema: lock_schema(),
            source: String::new(),
            index_sha256: None,
            ontologies: BTreeMap::new(),
        }
    }
}

impl LockFile {
    /// Loads `ontologies.lock.json` at `path`, or an empty default lock if
    /// the file does not exist yet (on-demand vendoring not yet adopted).
    ///
    /// # Errors
    ///
    /// Returns [`MifRhError::Io`] if `path` exists but cannot be read, or
    /// [`MifRhError::Json`] if it is not valid JSON.
    pub fn load_or_default(path: &Path) -> Result<Self, MifRhError> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let contents = std::fs::read_to_string(path).map_err(|source| MifRhError::Io {
            path: path.display().to_string(),
            source,
        })?;
        serde_json::from_str(&contents).map_err(|source| MifRhError::Json {
            path: path.display().to_string(),
            source,
        })
    }
}

/// Resolves the ontology registry source.
///
/// Mirrors rht's own precedence: an explicit override (e.g. the
/// `MIF_ONTOLOGY_SOURCE` env var, read by the CLI layer), then a
/// `.ontologies.source` marker file in `root`, then the canonical default.
#[must_use]
pub fn resolve_source(root: &Path, source_override: Option<&str>) -> String {
    if let Some(src) = source_override
        && !src.is_empty()
    {
        return trim_trailing_slash(src);
    }
    if let Ok(contents) = std::fs::read_to_string(root.join(".ontologies.source"))
        && let Some(first_line) = contents.lines().next()
        && !first_line.is_empty()
    {
        return trim_trailing_slash(first_line);
    }
    DEFAULT_REGISTRY_SOURCE.to_string()
}

fn trim_trailing_slash(source: &str) -> String {
    source.strip_suffix('/').unwrap_or(source).to_string()
}

fn is_http(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

/// Fetches one relative path from `source` (a local directory, `file://`
/// URL, or an http(s) base URL), returning its raw bytes.
fn fetch_raw(source: &str, relpath: &str) -> Result<Vec<u8>, MifRhError> {
    if is_http(source) {
        let url = format!("{source}/{relpath}");
        let mut response = ureq::get(&url)
            .call()
            .map_err(|err| MifRhError::RegistryFetch {
                registry_source: source.to_string(),
                detail: err.to_string(),
            })?;
        response
            .body_mut()
            .read_to_vec()
            .map_err(|err| MifRhError::RegistryFetch {
                registry_source: source.to_string(),
                detail: err.to_string(),
            })
    } else {
        let base = source.strip_prefix("file://").unwrap_or(source);
        let path = Path::new(base).join(relpath);
        std::fs::read(&path).map_err(|err| MifRhError::RegistryFetch {
            registry_source: source.to_string(),
            detail: format!("cannot read {}: {err}", path.display()),
        })
    }
}

/// Hex-encodes `bytes`' sha256 digest, lowercase, matching `sha256sum`'s
/// output format.
#[must_use]
fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// Whether `id` is already satisfied by a committed base layer under
/// `root/schemas/ontologies/<id>/` — never fetched, regardless of the
/// registry.
fn is_committed_base(root: &Path, id: &str) -> bool {
    root.join("schemas/ontologies").join(id).is_dir()
}

/// A bare, lowercase slug (matches `harness.config.schema.json`'s own
/// ontology id pattern) — rejects anything that could escape
/// `packs/ontologies/<id>/` once vendored.
fn is_wellformed_id(id: &str) -> bool {
    let mut chars = id.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Whether `file` (an index entry's attacker-controlled `file` field) is
/// safe to join onto a filesystem path: exactly one normal path component
/// (a bare filename). Checking only for `/` and `..` would still let a
/// Windows-style separator (`..\..\etc`) or an absolute path (`C:\...`)
/// through unchecked.
fn is_bare_filename(file: &str) -> bool {
    let mut components = std::path::Path::new(file).components();
    matches!(components.next(), Some(std::path::Component::Normal(_)))
        && components.next().is_none()
        && !file.contains('\\')
}

/// Resolves `requested`'s `extends` closure against `index` (breadth-first,
/// skipping committed base layers), returning the domain layers that must
/// actually be fetched, in discovery order.
///
/// # Errors
///
/// Returns [`MifRhError::OntologyNotInRegistry`] if a requested or ancestor
/// id has no index entry.
fn resolve_fetch_set(
    root: &Path,
    index: &RegistryIndex,
    requested: &[String],
) -> Result<Vec<String>, MifRhError> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut queue: Vec<String> = Vec::new();
    for id in requested {
        if seen.insert(id.clone()) {
            queue.push(id.clone());
        }
    }
    let mut fetch_list: Vec<String> = Vec::new();
    let mut cursor = 0;
    while cursor < queue.len() {
        let id = queue[cursor].clone();
        cursor += 1;
        // Every id reaching this point — whether directly requested or
        // discovered via a registry entry's `extends` — ends up joined onto
        // a filesystem path in `fetch()` (`packs_dir.join(id)`). A
        // compromised registry could otherwise smuggle a path-traversal id
        // (e.g. `../../../etc`) through `extends`, which is fully
        // attacker-controlled index content, the same threat model as the
        // `entry.file` bare-filename check below.
        if !is_wellformed_id(&id) {
            return Err(MifRhError::MalformedOntologyId { id });
        }
        if is_committed_base(root, &id) {
            continue;
        }
        let entry = index
            .ontologies
            .get(&id)
            .ok_or_else(|| MifRhError::OntologyNotInRegistry { id: id.clone() })?;
        if !fetch_list.contains(&id) {
            fetch_list.push(id.clone());
        }
        for ancestor in &entry.extends {
            if seen.insert(ancestor.clone()) {
                queue.push(ancestor.clone());
            }
        }
    }
    Ok(fetch_list)
}

/// One ontology vendored by [`fetch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VendoredOntology {
    /// The vendored ontology's id.
    pub id: String,
    /// The vendored version.
    pub version: String,
}

/// One ontology [`fetch`] left untouched at its pinned version.
///
/// Its `ontologies.lock.json` entry differs from the registry's current
/// version, and `refresh` was not set (research-harness-template#270: a
/// pinned corpus must not silently advance underneath an already-stamped
/// finding set).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedSkipped {
    /// The ontology's id.
    pub id: String,
    /// The version currently pinned in `ontologies.lock.json`.
    pub locked_version: String,
    /// The version the registry currently offers.
    pub registry_version: String,
}

/// The outcome of a [`fetch`] call.
#[derive(Debug, Clone, Default)]
pub struct FetchReport {
    /// Every ontology newly vendored or refreshed by this call
    /// (already-vendored, unchanged ontologies are not re-listed).
    pub vendored: Vec<VendoredOntology>,
    /// Every ontology left at its pinned version because the registry has
    /// moved past it and `refresh` was not set. Never populated on a first
    /// fetch of an id (nothing pinned yet to hold).
    pub pinned_skipped: Vec<PinnedSkipped>,
}

/// Whether `id` is already pinned in `lock` at a version different from
/// `entry`'s (the registry's current one) and `refresh` was not set — the
/// rht#270 drift `fetch` must hold rather than silently advance past.
fn pinned_below_registry(
    refresh: bool,
    lock: &LockFile,
    id: &str,
    entry: &IndexEntry,
) -> Option<PinnedSkipped> {
    if refresh {
        return None;
    }
    let locked = lock.ontologies.get(id)?;
    if locked.version == entry.version {
        return None;
    }
    Some(PinnedSkipped {
        id: id.to_string(),
        locked_version: locked.version.clone(),
        registry_version: entry.version.clone(),
    })
}

/// Vendors `ids` and their `extends` closure from the registry.
///
/// Fetches from `source`'s registry index into
/// `root/packs/ontologies/<id>/`, sha256-verifying every fetched file
/// against the index (fail-closed) and pinning the result in
/// `root/ontologies.lock.json`. A committed base layer under
/// `root/schemas/ontologies/<id>/` is always satisfied locally and never
/// fetched.
///
/// Trust-on-first-use, then pin: the index's own sha256 is recorded in the
/// lock on first fetch from a given `source`. A later fetch from the SAME
/// source whose index sha256 has changed is refused — the trust root
/// moved — unless the lock's `index_sha256` is cleared to re-pin
/// deliberately.
///
/// An id already present in `ontologies.lock.json` whose pinned version
/// differs from the registry's current one is left untouched unless
/// `refresh` is set — the lock IS the version pin (rht#270): a corpus must
/// not have an ontology's schema silently advance underneath its
/// already-stamped findings just because the registry published a newer
/// version. Skipped ids are reported in
/// [`FetchReport::pinned_skipped`], never silently dropped. An id with no
/// existing lock entry (a first-time fetch) is unaffected by `refresh` and
/// always vendors at the registry's current version.
///
/// # Errors
///
/// Returns [`MifRhError::RegistryFetch`] if the index or a file cannot be
/// read, [`MifRhError::RegistryIndexInvalid`] if the index is not valid
/// JSON, [`MifRhError::OntologyNotInRegistry`] if a requested or ancestor id
/// has no index entry, [`MifRhError::IndexPinMismatch`] if the source's
/// index sha256 no longer matches the lock's pinned value,
/// [`MifRhError::UnsafeIndexPath`] if the index names an unsafe file path,
/// [`MifRhError::ChecksumMismatch`] if a fetched file's sha256 does not
/// match the index, or [`MifRhError::Io`] if a vendored file cannot be
/// written or the lock cannot be saved.
pub fn fetch(
    root: &Path,
    source: &str,
    ids: &[String],
    refresh: bool,
) -> Result<FetchReport, MifRhError> {
    let index_bytes = fetch_raw(source, "index.json")?;
    let index_sha256 = sha256_hex(&index_bytes);
    let index: RegistryIndex =
        serde_json::from_slice(&index_bytes).map_err(|err| MifRhError::RegistryIndexInvalid {
            registry_source: source.to_string(),
            detail: err.to_string(),
        })?;

    let lock_path = root.join("ontologies.lock.json");
    let mut lock = LockFile::load_or_default(&lock_path)?;
    if let Some(pinned) = &lock.index_sha256
        && lock.source == source
        && *pinned != index_sha256
    {
        return Err(MifRhError::IndexPinMismatch {
            registry_source: source.to_string(),
            pinned: pinned.clone(),
            got: index_sha256,
        });
    }

    let fetch_list = resolve_fetch_set(root, &index, ids)?;
    let mut vendored = Vec::new();
    let mut pinned_skipped = Vec::new();
    let packs_dir = root.join("packs/ontologies");
    // Set before the loop (not after) and persisted incrementally per id
    // below: if a later id in this same request fails
    // (checksum/unsafe-path/IO), the ontologies successfully vendored
    // earlier in the loop must still end up pinned in the lock — otherwise
    // they sit on disk, sha256-verified once, permanently invisible to
    // `lock_check` (which only walks `lock.ontologies`, never the packs
    // directory).
    lock.index_sha256 = Some(index_sha256);
    lock.source = source.to_string();
    for id in &fetch_list {
        // Present in `index` by construction: `resolve_fetch_set` already
        // resolved every entry in `fetch_list` through a successful lookup.
        let Some(entry) = index.ontologies.get(id) else {
            continue;
        };
        if let Some(skip) = pinned_below_registry(refresh, &lock, id, entry) {
            pinned_skipped.push(skip);
            continue;
        }
        // `entry.file` is later joined onto a filesystem path, so it must
        // be a bare filename, never a path that could escape the intended
        // directory.
        if !is_bare_filename(&entry.file) {
            return Err(MifRhError::UnsafeIndexPath {
                id: id.clone(),
                file: entry.file.clone(),
            });
        }
        let bytes = fetch_raw(source, &entry.file)?;
        let got = sha256_hex(&bytes);
        if got != entry.sha256 {
            return Err(MifRhError::ChecksumMismatch {
                id: id.clone(),
                file: entry.file.clone(),
                expected: entry.sha256.clone(),
                got,
            });
        }
        let dest_dir = packs_dir.join(id);
        let out_yaml = dest_dir.join(format!("{id}.ontology.yaml"));
        // Parse BEFORE writing to disk: a parse failure must never leave a
        // malformed/wrong-shape YAML behind under packs/ontologies/ (that
        // would violate the fail-closed "no partial vendoring on error"
        // contract and confuse a later `lock_check`/`load_packs_via_catalog`
        // scan that finds the file but doesn't know it was never vendored).
        let pack = parse_pack(
            &String::from_utf8_lossy(&bytes),
            &out_yaml.display().to_string(),
        )?;
        std::fs::create_dir_all(&dest_dir).map_err(|source| MifRhError::Io {
            path: dest_dir.display().to_string(),
            source,
        })?;
        std::fs::write(&out_yaml, &bytes).map_err(|source| MifRhError::Io {
            path: out_yaml.display().to_string(),
            source,
        })?;
        let sidecar = serde_json::json!({
            "name": id,
            "version": entry.version,
            "kind": "ontology",
            "description": pack.description.as_deref().unwrap_or(""),
            "provides": {"ontologies": [id]},
        });
        crate::write_json_atomic(&dest_dir.join("ontology.pack.json"), &sidecar)?;

        lock.ontologies.insert(
            id.clone(),
            LockEntry {
                version: entry.version.clone(),
                sha256: entry.sha256.clone(),
            },
        );
        crate::write_json_atomic(&lock_path, &lock)?;
        vendored.push(VendoredOntology {
            id: id.clone(),
            version: entry.version.clone(),
        });
    }
    if vendored.is_empty() {
        // No per-id write happened above to persist the mutated
        // index_sha256/source: either fetch_list was empty (every requested
        // id was a committed base layer), or every entry in it was pinned
        // and skipped. Either way the pin/trust-root mutations made before
        // the loop still need to land on disk.
        crate::write_json_atomic(&lock_path, &lock)?;
    }

    Ok(FetchReport {
        vendored,
        pinned_skipped,
    })
}

/// Reads `harness.config.json`'s `.ontologies[]` (JSON, since `sync_registry`
/// must round-trip every OTHER top-level field of that file untouched).
fn load_config_json(path: &Path) -> Result<serde_json::Value, MifRhError> {
    if !path.exists() {
        return Err(MifRhError::ConfigMissing {
            path: path.display().to_string(),
        });
    }
    let contents = std::fs::read_to_string(path).map_err(|source| MifRhError::Io {
        path: path.display().to_string(),
        source,
    })?;
    serde_json::from_str(&contents).map_err(|source| MifRhError::Json {
        path: path.display().to_string(),
        source,
    })
}

fn ontologies_array(config: &serde_json::Value) -> &[serde_json::Value] {
    config
        .get("ontologies")
        .and_then(serde_json::Value::as_array)
        .map_or(&[][..], Vec::as_slice)
}

fn enabled_ontology_ids(config: &serde_json::Value) -> Vec<String> {
    ontologies_array(config)
        .iter()
        .filter(|entry| {
            entry
                .get("enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        })
        .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect()
}

fn known_ontology_ids(config: &serde_json::Value) -> HashSet<String> {
    ontologies_array(config)
        .iter()
        .filter_map(|entry| entry.get("id").and_then(serde_json::Value::as_str))
        .map(str::to_string)
        .collect()
}

/// One cataloged ontology entry written by [`sync_catalog`], matching
/// `sync-packs.sh`'s own `{id, version, source, core}` shape.
#[derive(Debug, Clone, Serialize)]
struct CatalogOntologyEntry {
    id: String,
    version: String,
    source: String,
    core: bool,
}

/// The outcome of a [`sync_catalog`] call.
#[derive(Debug, Clone, Default)]
pub struct CatalogSyncReport {
    /// How many ontologies (core + enabled) were cataloged.
    pub cataloged: usize,
}

/// Rebuilds the ontology-catalog section of rht's catalog sidecar.
///
/// Writes `.claude/enabled-packs.json`'s `.ontologies` key: every committed
/// base layer under `root/schemas/ontologies/` (core, if its id is one of
/// `mif-base`/`mif-generic`/`shared-traits`) plus every ontology enabled in
/// `harness.config.json` that is vendored under
/// `root/packs/ontologies/<id>/`.
///
/// Merges into whatever else already lives in `sidecar_path` (the general
/// pack/plugin-enablement half of `sync-packs.sh` writes other keys to the
/// same file) rather than overwriting it.
///
/// # Errors
///
/// Returns [`MifRhError::ConfigMissing`]/[`MifRhError::Json`] if
/// `config_path` cannot be read, [`MifRhError::Io`] if `root`'s ontology
/// directories cannot be read, [`MifRhError::OntologyPackYaml`] if a pack
/// fails to parse, and [`MifRhError::Io`]/[`MifRhError::JsonSerialize`] if
/// `sidecar_path` cannot be written.
pub fn sync_catalog(
    root: &Path,
    config_path: &Path,
    sidecar_path: &Path,
) -> Result<CatalogSyncReport, MifRhError> {
    let config = load_config_json(config_path)?;
    let mut entries = Vec::new();

    let core_dir = root.join("schemas/ontologies");
    if core_dir.is_dir() {
        let dir_entries = std::fs::read_dir(&core_dir).map_err(|source| MifRhError::Io {
            path: core_dir.display().to_string(),
            source,
        })?;
        let mut subdirs: Vec<_> = dir_entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        subdirs.sort();
        for subdir in subdirs {
            for pack_path in yaml_files_in(&subdir)? {
                let display = pack_path.display().to_string();
                let contents =
                    std::fs::read_to_string(&pack_path).map_err(|source| MifRhError::Io {
                        path: display.clone(),
                        source,
                    })?;
                let pack = parse_pack(&contents, &display)?;
                let core = CORE_IDS.contains(&pack.id.as_str());
                entries.push(CatalogOntologyEntry {
                    id: pack.id,
                    version: pack.version,
                    source: repo_relative(root, &pack_path),
                    core,
                });
            }
        }
    }

    let mut enabled: Vec<String> = enabled_ontology_ids(&config);
    enabled.sort();
    for id in enabled {
        let pack_path = root
            .join("packs/ontologies")
            .join(&id)
            .join(format!("{id}.ontology.yaml"));
        if !pack_path.is_file() {
            continue;
        }
        let display = pack_path.display().to_string();
        let contents = std::fs::read_to_string(&pack_path).map_err(|source| MifRhError::Io {
            path: display.clone(),
            source,
        })?;
        let pack = parse_pack(&contents, &display)?;
        entries.push(CatalogOntologyEntry {
            id: pack.id,
            version: pack.version,
            source: repo_relative(root, &pack_path),
            core: false,
        });
    }

    let mut sidecar = if sidecar_path.exists() {
        load_config_json(sidecar_path)?
    } else {
        serde_json::json!({})
    };
    let cataloged = entries.len();
    if let Some(object) = sidecar.as_object_mut() {
        object.insert(
            "ontologies".to_string(),
            serde_json::to_value(&entries).map_err(|source| MifRhError::JsonSerialize {
                path: sidecar_path.display().to_string(),
                source,
            })?,
        );
    }
    if let Some(parent) = sidecar_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|source| MifRhError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    crate::write_json_atomic(sidecar_path, &sidecar)?;

    Ok(CatalogSyncReport { cataloged })
}

fn yaml_files_in(dir: &Path) -> Result<Vec<std::path::PathBuf>, MifRhError> {
    let entries = std::fs::read_dir(dir).map_err(|source| MifRhError::Io {
        path: dir.display().to_string(),
        source,
    })?;
    let mut files: Vec<_> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
        })
        .collect();
    files.sort();
    Ok(files)
}

fn repo_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// The outcome of a [`sync_registry`] call.
#[derive(Debug, Clone, Default)]
pub struct RegistrySyncReport {
    /// Registry ontology ids discovered and newly added (enabled by
    /// default) to `harness.config.json`.
    pub discovered: Vec<String>,
    /// The subsequent vendor pass over every now-enabled ontology.
    pub fetch: FetchReport,
}

/// Discovers domain ontologies newly published to the registry.
///
/// Finds ontologies in `source`'s registry that `config_path` has never
/// heard of (not merely disabled — absent from `.ontologies[]` entirely),
/// adds each with `enabled: true` (this harness's default posture), then
/// vendors and catalogs everything currently enabled.
///
/// # Errors
///
/// Returns [`MifRhError::ConfigMissing`]/[`MifRhError::Json`] if
/// `config_path` cannot be read, [`MifRhError::RegistryFetch`]/
/// [`MifRhError::RegistryIndexInvalid`] if the registry index cannot be
/// read or parsed, [`MifRhError::MalformedOntologyId`] if the index
/// declares a malformed id, [`MifRhError::ConfigMalformed`] if
/// `config_path`'s `.ontologies` exists but is not an array, or any error
/// [`fetch`]/[`sync_catalog`] can return.
pub fn sync_registry(
    root: &Path,
    config_path: &Path,
    sidecar_path: &Path,
    source: &str,
) -> Result<RegistrySyncReport, MifRhError> {
    // Validate the config's shape BEFORE any network/source activity — a
    // malformed config must fail fast and offline, matching
    // `sync-registry-ontologies.sh`'s own original `jq -e '.ontologies |
    // type == "array"'` guard, which ran before it ever touched the
    // registry.
    let mut config = load_config_json(config_path)?;
    if !config
        .get("ontologies")
        .is_some_and(serde_json::Value::is_array)
    {
        return Err(MifRhError::ConfigMalformed {
            path: config_path.display().to_string(),
            detail: ".ontologies is missing or not an array".to_string(),
        });
    }

    let index_bytes = fetch_raw(source, "index.json")?;
    let index: RegistryIndex =
        serde_json::from_slice(&index_bytes).map_err(|err| MifRhError::RegistryIndexInvalid {
            registry_source: source.to_string(),
            detail: err.to_string(),
        })?;
    // Check the trust-on-first-use pin BEFORE mutating harness.config.json
    // below: `fetch()` (called later in this function) re-fetches and
    // re-checks the same index independently, but if that later check
    // fails, config.json must not have already been durably rewritten to
    // enable ontologies discovered from an index whose trust root moved.
    let lock = LockFile::load_or_default(&root.join("ontologies.lock.json"))?;
    if let Some(pinned) = &lock.index_sha256
        && lock.source == source
        && *pinned != sha256_hex(&index_bytes)
    {
        return Err(MifRhError::IndexPinMismatch {
            registry_source: source.to_string(),
            pinned: pinned.clone(),
            got: sha256_hex(&index_bytes),
        });
    }

    let known = known_ontology_ids(&config);
    let mut discovered: Vec<String> = index
        .ontologies
        .keys()
        .filter(|id| !is_committed_base(root, id) && !known.contains(*id))
        .cloned()
        .collect();
    discovered.sort();
    for id in &discovered {
        if !is_wellformed_id(id) {
            return Err(MifRhError::MalformedOntologyId { id: id.clone() });
        }
    }

    if !discovered.is_empty() {
        let object = config
            .as_object_mut()
            .ok_or_else(|| MifRhError::ConfigMalformed {
                path: config_path.display().to_string(),
                detail: "top-level document is not a JSON object".to_string(),
            })?;
        let array = object
            .entry("ontologies")
            .or_insert_with(|| serde_json::Value::Array(Vec::new()))
            .as_array_mut()
            .ok_or_else(|| MifRhError::ConfigMalformed {
                path: config_path.display().to_string(),
                detail: ".ontologies exists but is not an array".to_string(),
            })?;
        for id in &discovered {
            array.push(serde_json::json!({"id": id, "enabled": true}));
        }
        crate::write_json_atomic(config_path, &config)?;
    }

    let enabled = enabled_ontology_ids(&config);
    // `refresh: false` — sync-registry's job is discovering NEWLY published
    // ids and enabling them, not silently advancing ids already pinned in
    // ontologies.lock.json (rht#270). A newly-discovered id has no lock
    // entry yet, so it always vendors regardless of this flag.
    let fetch_report = fetch(root, source, &enabled, false)?;
    sync_catalog(root, config_path, sidecar_path)?;

    Ok(RegistrySyncReport {
        discovered,
        fetch: fetch_report,
    })
}

/// One drifted vendored ontology: its id, its pinned sha256, and the
/// sha256 actually found on disk.
#[derive(Debug, Clone)]
pub struct DriftEntry {
    /// The drifted ontology's id.
    pub id: String,
    /// The pinned (expected) sha256.
    pub pinned: String,
    /// The sha256 actually computed from the file on disk.
    pub got: String,
}

/// The outcome of a [`lock_check`] call.
#[derive(Debug, Clone, Default)]
pub struct LockCheckReport {
    /// Enabled ontology ids with no pin in `ontologies.lock.json`.
    pub missing_pins: Vec<String>,
    /// Enabled, pinned ontology ids with no vendored file on disk.
    pub not_vendored: Vec<String>,
    /// Pinned, vendored ontologies whose on-disk sha256 no longer matches
    /// the pin.
    pub drift: Vec<DriftEntry>,
    /// How many vendored ontologies were checked and matched their pin.
    pub checked: usize,
}

impl LockCheckReport {
    /// Whether every check passed: no missing pins, no un-vendored enabled
    /// ontologies, and no drift.
    #[must_use]
    pub const fn ok(&self) -> bool {
        self.missing_pins.is_empty() && self.not_vendored.is_empty() && self.drift.is_empty()
    }
}

/// Proves vendored domain ontologies match `root/ontologies.lock.json`.
///
/// Checks coverage (every enabled domain ontology is pinned and vendored)
/// and integrity (every vendored ontology present on disk hashes to its
/// pinned sha256). A missing lock file is not an error — on-demand
/// vendoring has not been adopted in the clone, so there is nothing to
/// verify.
///
/// # Errors
///
/// Returns [`MifRhError::ConfigMissing`]/[`MifRhError::Json`] if
/// `config_path` cannot be read, or [`MifRhError::Io`]/[`MifRhError::Json`]
/// if the lock file exists but cannot be read/parsed.
pub fn lock_check(root: &Path, config_path: &Path) -> Result<LockCheckReport, MifRhError> {
    let lock_path = root.join("ontologies.lock.json");
    if !lock_path.exists() {
        return Ok(LockCheckReport::default());
    }
    let lock = LockFile::load_or_default(&lock_path)?;
    let config = load_config_json(config_path)?;

    let mut report = LockCheckReport::default();
    // A BTreeSet, not a HashSet: `missing_pins` is built by iterating this
    // set, and its order should stay deterministic rather than depend on
    // hash iteration order.
    let enabled: BTreeSet<String> = enabled_ontology_ids(&config).into_iter().collect();

    for id in &enabled {
        if is_committed_base(root, id) {
            continue;
        }
        if !lock.ontologies.contains_key(id) {
            report.missing_pins.push(id.clone());
        }
    }

    for (id, entry) in &lock.ontologies {
        let yaml = root
            .join("packs/ontologies")
            .join(id)
            .join(format!("{id}.ontology.yaml"));
        if !yaml.is_file() {
            if enabled.contains(id) {
                report.not_vendored.push(id.clone());
            }
            continue;
        }
        let bytes = std::fs::read(&yaml).map_err(|source| MifRhError::Io {
            path: yaml.display().to_string(),
            source,
        })?;
        let got = sha256_hex(&bytes);
        if got == entry.sha256 {
            report.checked += 1;
        } else {
            report.drift.push(DriftEntry {
                id: id.clone(),
                pinned: entry.sha256.clone(),
                got,
            });
        }
    }

    Ok(report)
}

/// One `required` field the registry's advanced schema newly demands for an
/// entity type both the vendored and registry packs declare.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewlyRequiredField {
    /// The entity type the field was added to.
    pub entity_type: String,
    /// The newly required field's name.
    pub field: String,
}

/// A stamped finding missing a newly required field.
///
/// The breaking-change warning [`check_pin_safety`] exists to surface,
/// distinct from `fetch`'s plain version-drift warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinSafetyGap {
    /// The topic the finding belongs to.
    pub topic: String,
    /// The finding's id.
    pub finding_id: String,
    /// The finding's stamped entity type.
    pub entity_type: String,
    /// The newly required field the finding's `entity` payload lacks.
    pub field: String,
}

/// One pinned, drifted ontology's pin-safety analysis.
#[derive(Debug, Clone, Default)]
pub struct PinSafetyReport {
    /// The ontology's id.
    pub id: String,
    /// The version currently pinned in `ontologies.lock.json`.
    pub locked_version: String,
    /// The version the registry currently offers.
    pub registry_version: String,
    /// Whether the diff was actually performed. `false` when the vendored
    /// pack could not be read from disk to diff against — in that case
    /// `newly_required`/`gaps` are empty because nothing was analyzed, NOT
    /// because the drift was found safe. Callers must check this before
    /// treating an empty `gaps` as a clean bill of health.
    pub analyzed: bool,
    /// Fields the registry's advanced schema newly requires, relative to
    /// the vendored version. Always empty when `analyzed` is `false`.
    pub newly_required: Vec<NewlyRequiredField>,
    /// Stamped findings missing a newly required field. Always empty when
    /// `newly_required` is empty.
    pub gaps: Vec<PinSafetyGap>,
}

/// The `required` array of an entity type's `schema` object, as strings.
///
/// # Errors
///
/// Returns [`MifRhError::EntityTypeSchemaInvalid`] if `required` is present
/// but is not an array of strings. A missing `required` key is not an
/// error — it means the entity type declares no required fields — but a
/// PRESENT, malformed one must fail closed rather than silently default to
/// an empty list: for a pin-safety check, treating a malformed schema as
/// "nothing required" would produce a false "safe" verdict, the
/// worst-case failure mode for this specific check.
fn required_fields(
    entity_type: &str,
    schema: &serde_json::Value,
) -> Result<Vec<String>, MifRhError> {
    let Some(required) = schema.get("required") else {
        return Ok(Vec::new());
    };
    let Some(values) = required.as_array() else {
        return Err(MifRhError::EntityTypeSchemaInvalid {
            entity_type: entity_type.to_string(),
            detail: "schema.required is present but is not an array".to_string(),
        });
    };
    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| MifRhError::EntityTypeSchemaInvalid {
                    entity_type: entity_type.to_string(),
                    detail: format!("schema.required contains a non-string element: {value}"),
                })
        })
        .collect()
}

/// The `required` fields `new`'s schema adds for an entity type present in
/// both `old` and `new`. An entity type absent from `old` is skipped
/// entirely: it is wholly new, so no existing stamped finding could have
/// been typed with it, and nothing about it can be "newly" required.
///
/// # Errors
///
/// Returns [`MifRhError::EntityTypeSchemaInvalid`] if either pack's
/// `schema.required` is malformed; see [`required_fields`].
fn diff_newly_required(
    old: &crate::ontology_pack::OntologyPack,
    new: &crate::ontology_pack::OntologyPack,
) -> Result<Vec<NewlyRequiredField>, MifRhError> {
    let mut out = Vec::new();
    for new_type in &new.entity_types {
        let Some(old_type) = old.entity_types.iter().find(|t| t.name == new_type.name) else {
            continue;
        };
        let old_required: HashSet<String> = required_fields(&old_type.name, &old_type.schema)?
            .into_iter()
            .collect();
        let mut seen_new = HashSet::new();
        for field in required_fields(&new_type.name, &new_type.schema)? {
            if !old_required.contains(&field) && seen_new.insert(field.clone()) {
                out.push(NewlyRequiredField {
                    entity_type: new_type.name.clone(),
                    field,
                });
            }
        }
    }
    Ok(out)
}

/// Cross-references `newly_required` against every stamped finding (basis
/// declared/resolved and valid — the same predicate `collect_topic_samples`
/// uses) across `topics` whose `resolved_ontology` names `ontology_id`,
/// reporting one [`PinSafetyGap`] per finding whose `entity` payload lacks a
/// newly required field.
fn find_pin_safety_gaps(
    reports_dir: &Path,
    topics: &[String],
    ontology_id: &str,
    newly_required: &[NewlyRequiredField],
) -> Result<Vec<PinSafetyGap>, MifRhError> {
    let mut gaps = Vec::new();
    for topic in topics {
        let map_path = reports_dir.join(topic).join("ontology-map.json");
        let contents = match std::fs::read_to_string(&map_path) {
            Ok(contents) => contents,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => continue,
            Err(source) => {
                return Err(MifRhError::Io {
                    path: map_path.display().to_string(),
                    source,
                });
            },
        };
        let records: Vec<crate::resolve::MapRecord> =
            serde_json::from_str(&contents).map_err(|source| MifRhError::Json {
                path: map_path.display().to_string(),
                source,
            })?;
        // Indexed once per topic rather than linearly scanned per finding
        // file below (O(records) once vs. O(records) per finding).
        let records_by_finding_id: std::collections::HashMap<&str, &crate::resolve::MapRecord> =
            records.iter().map(|r| (r.finding_id.as_str(), r)).collect();

        let findings_dir = reports_dir.join(topic).join("findings");
        if !findings_dir.is_dir() {
            continue;
        }
        for file in crate::review::list_finding_files(&findings_dir)? {
            let Ok(finding) = crate::finding::Finding::load(&file) else {
                continue; // gap findings are review's concern, not this one's
            };
            let Some(record) = records_by_finding_id.get(finding.id.as_str()).copied() else {
                continue;
            };
            let stamped = record.valid
                && matches!(
                    record.basis,
                    crate::resolve::Basis::Declared | crate::resolve::Basis::Resolved
                );
            if !stamped {
                continue;
            }
            let bound_to_this_ontology = record
                .resolved_ontology
                .as_deref()
                .and_then(|resolved| resolved.split('@').next())
                == Some(ontology_id);
            if !bound_to_this_ontology {
                continue;
            }
            let Some(entity_type) = record.entity_type.as_deref() else {
                continue;
            };
            gaps.extend(
                newly_required
                    .iter()
                    .filter(|nrf| nrf.entity_type == entity_type)
                    .filter(|nrf| {
                        finding
                            .entity
                            .as_ref()
                            .and_then(|entity| entity.get(&nrf.field))
                            .is_none_or(serde_json::Value::is_null)
                    })
                    .map(|nrf| PinSafetyGap {
                        topic: topic.clone(),
                        finding_id: finding.id.clone(),
                        entity_type: entity_type.to_string(),
                        field: nrf.field.clone(),
                    }),
            );
        }
    }
    Ok(gaps)
}

/// Warns only when a pinned ontology's advanced schema actually breaks an
/// already-stamped finding.
///
/// For each of `ids` currently pinned in `root/ontologies.lock.json` whose
/// locked version differs from the registry's current one, diffs the
/// vendored (old) and registry (new) schema's `entity_types[].schema.required`
/// lists, and cross-references any newly required field against `topics`'
/// already-stamped findings — the narrower, smarter follow-up to `fetch`'s
/// plain version-drift warning (research-harness-template#270's proposed
/// fix #2; tracked as mif-rs#61).
///
/// An id with no lock entry, or already at the registry's current version,
/// produces no report entry — nothing pinned, or nothing drifted, to
/// analyze. An id whose vendored pack cannot be read from disk (e.g.
/// deleted since the pin was recorded) produces a report entry with empty
/// `newly_required`/`gaps`: the version drift is still real, but nothing
/// can be diffed against.
///
/// Read-only: never touches `ontologies.lock.json`, never vendors, never
/// advances a pin. Downloads the registry's current file for a drifted id
/// purely to diff it — unlike `fetch`, which never re-fetches a
/// pinned-skipped id's file at all — applying the same integrity checks
/// `fetch` applies to that same attacker-controlled path (index-pin
/// verification, bare-filename validation, sha256 verification against the
/// index) before trusting any fetched byte.
///
/// # Errors
///
/// Returns [`MifRhError::MalformedOntologyId`] if a requested id is not a
/// bare, lowercase slug, [`MifRhError::RegistryFetch`] if the registry
/// index or a drifted id's file cannot be fetched,
/// [`MifRhError::RegistryIndexInvalid`] if the index is not valid JSON,
/// [`MifRhError::IndexPinMismatch`] if the source's index sha256 no longer
/// matches the lock's pinned value, [`MifRhError::OntologyNotInRegistry`]
/// if a requested id has no index entry, [`MifRhError::UnsafeIndexPath`] if
/// the index names an unsafe file path, [`MifRhError::ChecksumMismatch`] if
/// a fetched file's sha256 does not match the index,
/// [`MifRhError::OntologyPackYaml`] if the vendored or registry pack YAML
/// is malformed, or [`MifRhError::Io`]/[`MifRhError::Json`] if a topic's
/// `ontology-map.json` cannot be read or parsed. An individual finding
/// file that cannot be read or parsed is silently skipped rather than
/// propagated (the same treatment `collect_topic_samples` gives an
/// unreadable finding) — a gap-analysis finding is review's concern, not
/// this function's.
pub fn check_pin_safety(
    root: &Path,
    source: &str,
    reports_dir: &Path,
    topics: &[String],
    ids: &[String],
) -> Result<Vec<PinSafetyReport>, MifRhError> {
    let index_bytes = fetch_raw(source, "index.json")?;
    let index_sha256 = sha256_hex(&index_bytes);
    let index: RegistryIndex =
        serde_json::from_slice(&index_bytes).map_err(|err| MifRhError::RegistryIndexInvalid {
            registry_source: source.to_string(),
            detail: err.to_string(),
        })?;
    let lock = LockFile::load_or_default(&root.join("ontologies.lock.json"))?;
    // Same trust-on-first-use check `fetch` enforces before trusting index
    // content (`vendor.rs`'s module doc: the index is "fully
    // attacker-controlled"). Read-only here — never re-pins, only refuses
    // to proceed on a mismatch.
    if let Some(pinned) = &lock.index_sha256
        && lock.source == source
        && *pinned != index_sha256
    {
        return Err(MifRhError::IndexPinMismatch {
            registry_source: source.to_string(),
            pinned: pinned.clone(),
            got: index_sha256,
        });
    }

    let mut reports = Vec::new();
    for id in ids {
        // `id` is joined onto a filesystem path below (`old_path`), so it
        // must be validated the same way `resolve_fetch_set` validates
        // every id (including directly-requested ones) before any use —
        // a caller-supplied id is just as untrusted a path component as
        // one discovered via the registry's `extends` chain.
        if !is_wellformed_id(id) {
            return Err(MifRhError::MalformedOntologyId { id: id.clone() });
        }
        let Some(locked) = lock.ontologies.get(id) else {
            continue; // nothing pinned, nothing to check
        };
        let entry = index
            .ontologies
            .get(id)
            .ok_or_else(|| MifRhError::OntologyNotInRegistry { id: id.clone() })?;
        if locked.version == entry.version {
            continue; // not drifted, nothing to check
        }

        let old_path = root
            .join("packs/ontologies")
            .join(id)
            .join(format!("{id}.ontology.yaml"));
        let old_pack = match std::fs::read_to_string(&old_path) {
            Ok(yaml) => Some(parse_pack(&yaml, &old_path.display().to_string())?),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
            Err(source) => {
                return Err(MifRhError::Io {
                    path: old_path.display().to_string(),
                    source,
                });
            },
        };
        let Some(old_pack) = old_pack else {
            reports.push(PinSafetyReport {
                id: id.clone(),
                locked_version: locked.version.clone(),
                registry_version: entry.version.clone(),
                analyzed: false,
                newly_required: Vec::new(),
                gaps: Vec::new(),
            });
            continue;
        };

        // Same integrity checks `fetch` enforces on this exact
        // attacker-controlled path before trusting a fetched file:
        // `entry.file` must be a bare filename (never a path that could
        // escape the intended directory), and the downloaded bytes must
        // match the index's own pinned sha256.
        if !is_bare_filename(&entry.file) {
            return Err(MifRhError::UnsafeIndexPath {
                id: id.clone(),
                file: entry.file.clone(),
            });
        }
        let new_bytes = fetch_raw(source, &entry.file)?;
        let got = sha256_hex(&new_bytes);
        if got != entry.sha256 {
            return Err(MifRhError::ChecksumMismatch {
                id: id.clone(),
                file: entry.file.clone(),
                expected: entry.sha256.clone(),
                got,
            });
        }
        let new_pack = parse_pack(
            &String::from_utf8_lossy(&new_bytes),
            &format!("{source}/{}", entry.file),
        )?;

        let newly_required = diff_newly_required(&old_pack, &new_pack)?;
        let gaps = if newly_required.is_empty() {
            Vec::new()
        } else {
            find_pin_safety_gaps(reports_dir, topics, id, &newly_required)?
        };

        reports.push(PinSafetyReport {
            id: id.clone(),
            locked_version: locked.version.clone(),
            registry_version: entry.version.clone(),
            analyzed: true,
            newly_required,
            gaps,
        });
    }
    Ok(reports)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        DEFAULT_REGISTRY_SOURCE, diff_newly_required, fetch, is_wellformed_id, lock_check,
        resolve_source, sync_catalog, sync_registry,
    };
    use crate::ontology_pack::parse_pack;

    const EDU_INDEX: &str = r#"{
        "ontologies": {
            "edu-fixture": {
                "version": "0.1.0",
                "sha256": "REPLACED",
                "file": "edu-fixture.ontology.yaml",
                "extends": ["mif-base"]
            }
        }
    }"#;

    const EDU_YAML: &str = "ontology:\n  id: edu-fixture\n  version: \"0.1.0\"\n  description: \"An edu fixture\"\n  extends: [mif-base]\nentity_types: []\n";

    fn sha256_of(bytes: &[u8]) -> String {
        super::sha256_hex(bytes)
    }

    /// Writes a local-directory registry source (index.json + one ontology
    /// file) under `dir/registry/`, with the index's sha256 filled in for
    /// real, and returns the source path as a string.
    fn write_local_registry(dir: &std::path::Path) -> String {
        let registry = dir.join("registry");
        fs::create_dir_all(&registry).unwrap();
        let sha = sha256_of(EDU_YAML.as_bytes());
        let index = EDU_INDEX.replace("REPLACED", &sha);
        fs::write(registry.join("index.json"), index).unwrap();
        fs::write(registry.join("edu-fixture.ontology.yaml"), EDU_YAML).unwrap();
        registry.display().to_string()
    }

    fn write_base_layer(root: &std::path::Path) {
        fs::create_dir_all(root.join("schemas/ontologies/mif-base")).unwrap();
        fs::write(
            root.join("schemas/ontologies/mif-base/mif-base.ontology.yaml"),
            "ontology:\n  id: mif-base\n  version: \"1.0.0\"\nentity_types: []\n",
        )
        .unwrap();
    }

    #[test]
    fn resolve_source_defaults_to_the_canonical_registry() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resolve_source(dir.path(), None), DEFAULT_REGISTRY_SOURCE);
    }

    #[test]
    fn resolve_source_prefers_an_explicit_override() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".ontologies.source"), "/marker/path\n").unwrap();
        assert_eq!(resolve_source(dir.path(), Some("/override/")), "/override");
    }

    #[test]
    fn resolve_source_falls_back_to_the_marker_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".ontologies.source"), "/marker/path/\n").unwrap();
        assert_eq!(resolve_source(dir.path(), None), "/marker/path");
    }

    #[test]
    fn is_wellformed_id_accepts_bare_lowercase_slugs_only() {
        assert!(is_wellformed_id("clinical-trials"));
        assert!(is_wellformed_id("edu2"));
        assert!(!is_wellformed_id(""));
        assert!(!is_wellformed_id("../etc"));
        assert!(!is_wellformed_id("Clinical"));
        assert!(!is_wellformed_id("has space"));
    }

    #[test]
    fn fetch_vendors_a_verified_ontology_and_skips_committed_bases() {
        let dir = tempfile::tempdir().unwrap();
        write_base_layer(dir.path());
        let source = write_local_registry(dir.path());

        let report = fetch(dir.path(), &source, &["edu-fixture".to_string()], false).unwrap();

        assert_eq!(report.vendored.len(), 1);
        assert_eq!(report.vendored[0].id, "edu-fixture");
        let vendored_yaml = dir
            .path()
            .join("packs/ontologies/edu-fixture/edu-fixture.ontology.yaml");
        assert!(vendored_yaml.is_file());
        let lock: super::LockFile = serde_json::from_str(
            &fs::read_to_string(dir.path().join("ontologies.lock.json")).unwrap(),
        )
        .unwrap();
        assert!(lock.ontologies.contains_key("edu-fixture"));
        assert!(lock.index_sha256.is_some());
        // mif-base is a committed base layer: it must never be vendored,
        // even though edu-fixture's extends names it.
        assert!(!dir.path().join("packs/ontologies/mif-base").exists());
    }

    /// Writes a registry at `version` for `edu-fixture` (distinct content
    /// from `write_local_registry`'s 0.1.0, so its sha256 genuinely
    /// differs), then seeds `ontologies.lock.json` claiming `edu-fixture`
    /// is already pinned at `locked_version` — an already-index-trusted
    /// registry (the index sha256 in the lock matches the one on disk) that
    /// has moved past a per-id pin, the exact drift `fetch`'s pin-respecting
    /// check (rht#270) must catch. Returns the registry source path.
    fn seed_registry_ahead_of_a_pinned_lock(
        dir: &std::path::Path,
        registry_version: &str,
        locked_version: &str,
    ) -> String {
        let registry = dir.join("registry");
        fs::create_dir_all(&registry).unwrap();
        let yaml = format!(
            "ontology:\n  id: edu-fixture\n  version: \"{registry_version}\"\n  description: \
             \"An edu fixture at {registry_version}\"\n  extends: [mif-base]\nentity_types: \
             []\n"
        );
        let sha = sha256_of(yaml.as_bytes());
        let index = format!(
            r#"{{"ontologies":{{"edu-fixture":{{"version":"{registry_version}","sha256":"{sha}","file":"edu-fixture.ontology.yaml","extends":["mif-base"]}}}}}}"#
        );
        fs::write(registry.join("index.json"), &index).unwrap();
        fs::write(registry.join("edu-fixture.ontology.yaml"), yaml).unwrap();

        let source = registry.display().to_string();
        let lock = serde_json::json!({
            "schema": "mif-ontology-lock/v1",
            "source": source,
            "index_sha256": sha256_of(index.as_bytes()),
            "ontologies": {
                "edu-fixture": {"version": locked_version, "sha256": "irrelevant-for-this-test"}
            }
        });
        fs::write(
            dir.join("ontologies.lock.json"),
            serde_json::to_string(&lock).unwrap(),
        )
        .unwrap();
        source
    }

    #[test]
    fn fetch_leaves_a_pinned_ontology_untouched_when_the_registry_advances_without_refresh() {
        let dir = tempfile::tempdir().unwrap();
        write_base_layer(dir.path());
        let source = seed_registry_ahead_of_a_pinned_lock(dir.path(), "0.2.0", "0.1.0");

        let report = fetch(dir.path(), &source, &["edu-fixture".to_string()], false).unwrap();

        assert!(report.vendored.is_empty());
        assert_eq!(report.pinned_skipped.len(), 1);
        assert_eq!(report.pinned_skipped[0].id, "edu-fixture");
        assert_eq!(report.pinned_skipped[0].locked_version, "0.1.0");
        assert_eq!(report.pinned_skipped[0].registry_version, "0.2.0");

        let lock: super::LockFile = serde_json::from_str(
            &fs::read_to_string(dir.path().join("ontologies.lock.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(lock.ontologies["edu-fixture"].version, "0.1.0");
        assert!(
            !dir.path()
                .join("packs/ontologies/edu-fixture/edu-fixture.ontology.yaml")
                .exists(),
            "a pinned-and-skipped id must never be written to disk"
        );
    }

    #[test]
    fn fetch_refresh_advances_a_pinned_ontology_to_the_registry_version() {
        let dir = tempfile::tempdir().unwrap();
        write_base_layer(dir.path());
        let source = seed_registry_ahead_of_a_pinned_lock(dir.path(), "0.2.0", "0.1.0");

        let report = fetch(dir.path(), &source, &["edu-fixture".to_string()], true).unwrap();

        assert!(report.pinned_skipped.is_empty());
        assert_eq!(report.vendored.len(), 1);
        assert_eq!(report.vendored[0].version, "0.2.0");

        let lock: super::LockFile = serde_json::from_str(
            &fs::read_to_string(dir.path().join("ontologies.lock.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(lock.ontologies["edu-fixture"].version, "0.2.0");
        assert!(
            dir.path()
                .join("packs/ontologies/edu-fixture/edu-fixture.ontology.yaml")
                .exists()
        );
    }

    #[test]
    fn fetch_persists_a_cleared_index_pin_even_when_every_id_is_pinned_skipped() {
        // Regression: when every requested id is pinned-and-skipped, no
        // per-id write happens in the loop, so the index_sha256/source
        // mutated before the loop must still be persisted by the
        // fallback write after it — otherwise a deliberate re-pin (clear
        // index_sha256, re-fetch) silently fails to record the new trust
        // root whenever it also happens to hit only already-pinned ids.
        let dir = tempfile::tempdir().unwrap();
        write_base_layer(dir.path());
        let source = seed_registry_ahead_of_a_pinned_lock(dir.path(), "0.2.0", "0.1.0");

        // Clear index_sha256, simulating a deliberate re-pin in progress.
        let mut lock: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.path().join("ontologies.lock.json")).unwrap(),
        )
        .unwrap();
        lock.as_object_mut().unwrap().remove("index_sha256");
        fs::write(
            dir.path().join("ontologies.lock.json"),
            serde_json::to_string(&lock).unwrap(),
        )
        .unwrap();

        let report = fetch(dir.path(), &source, &["edu-fixture".to_string()], false).unwrap();
        assert!(report.vendored.is_empty());
        assert_eq!(report.pinned_skipped.len(), 1);

        let after: super::LockFile = serde_json::from_str(
            &fs::read_to_string(dir.path().join("ontologies.lock.json")).unwrap(),
        )
        .unwrap();
        assert!(
            after.index_sha256.is_some(),
            "the re-pinned index_sha256 must be persisted even though every \
             requested id was pinned-skipped"
        );
    }

    #[test]
    fn fetch_fails_closed_on_a_checksum_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        write_base_layer(dir.path());
        let registry = dir.path().join("registry");
        fs::create_dir_all(&registry).unwrap();
        let index = EDU_INDEX.replace(
            "REPLACED",
            "0000000000000000000000000000000000000000000000000000000000000000",
        );
        fs::write(registry.join("index.json"), index).unwrap();
        fs::write(registry.join("edu-fixture.ontology.yaml"), EDU_YAML).unwrap();

        let error = fetch(
            dir.path(),
            &registry.display().to_string(),
            &["edu-fixture".to_string()],
            false,
        )
        .unwrap_err();
        assert!(matches!(error, super::MifRhError::ChecksumMismatch { .. }));
        assert!(!dir.path().join("packs/ontologies/edu-fixture").exists());
    }

    #[test]
    fn fetch_rejects_a_windows_style_or_absolute_index_file_path() {
        let dir = tempfile::tempdir().unwrap();
        write_base_layer(dir.path());
        let registry = dir.path().join("registry");
        fs::create_dir_all(&registry).unwrap();
        for unsafe_file in [
            "..\\..\\etc\\passwd",
            "C:\\Windows\\evil.yaml",
            "/etc/passwd",
        ] {
            // Substitute via a JSON-escaped literal (not a raw string replace)
            // so a literal backslash in `unsafe_file` doesn't itself produce
            // invalid JSON.
            let index = EDU_INDEX
                .replace("REPLACED", &sha256_of(EDU_YAML.as_bytes()))
                .replace(
                    "\"edu-fixture.ontology.yaml\"",
                    &serde_json::to_string(unsafe_file).unwrap(),
                );
            fs::write(registry.join("index.json"), index).unwrap();

            let error = fetch(
                dir.path(),
                &registry.display().to_string(),
                &["edu-fixture".to_string()],
                false,
            )
            .unwrap_err();
            assert!(
                matches!(error, super::MifRhError::UnsafeIndexPath { .. }),
                "expected UnsafeIndexPath for {unsafe_file:?}, got {error:?}"
            );
        }
    }

    #[test]
    fn fetch_leaves_no_file_behind_when_the_vendored_yaml_fails_to_parse() {
        let dir = tempfile::tempdir().unwrap();
        write_base_layer(dir.path());
        let registry = dir.path().join("registry");
        fs::create_dir_all(&registry).unwrap();
        let malformed = b"not: [valid, yaml, ontology shape";
        let index = EDU_INDEX.replace("REPLACED", &sha256_of(malformed));
        fs::write(registry.join("index.json"), index).unwrap();
        fs::write(registry.join("edu-fixture.ontology.yaml"), malformed).unwrap();

        let error = fetch(
            dir.path(),
            &registry.display().to_string(),
            &["edu-fixture".to_string()],
            false,
        )
        .unwrap_err();

        assert!(
            matches!(error, super::MifRhError::OntologyPackYaml { .. }),
            "{error:?}"
        );
        assert!(!dir.path().join("packs/ontologies/edu-fixture").exists());
    }

    #[test]
    fn fetch_refuses_a_moved_trust_root_for_the_same_source() {
        let dir = tempfile::tempdir().unwrap();
        write_base_layer(dir.path());
        let source = write_local_registry(dir.path());
        fetch(dir.path(), &source, &["edu-fixture".to_string()], false).unwrap();

        // The same source's index.json now hashes differently.
        fs::write(
            std::path::Path::new(&source).join("index.json"),
            EDU_INDEX
                .replace("REPLACED", &sha256_of(EDU_YAML.as_bytes()))
                .replace('0', "1"),
        )
        .unwrap();

        let error = fetch(dir.path(), &source, &["edu-fixture".to_string()], false).unwrap_err();
        assert!(matches!(error, super::MifRhError::IndexPinMismatch { .. }));
    }

    #[test]
    fn fetch_reports_an_id_absent_from_the_registry() {
        let dir = tempfile::tempdir().unwrap();
        let source = write_local_registry(dir.path());

        let error = fetch(dir.path(), &source, &["nonexistent-id".to_string()], false).unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::OntologyNotInRegistry { .. }
        ));
    }

    #[test]
    fn fetch_rejects_a_path_traversal_id_reached_via_an_extends_ancestor() {
        // A compromised registry can name anything in an `extends` array —
        // it is fully attacker-controlled index content, just like a
        // requested id. A malformed ancestor id must be rejected before it
        // is ever joined onto a filesystem path, not just at discovery time
        // in `sync_registry`.
        let dir = tempfile::tempdir().unwrap();
        let registry = dir.path().join("registry");
        fs::create_dir_all(&registry).unwrap();
        let sha = sha256_of(EDU_YAML.as_bytes());
        let malicious_index = format!(
            r#"{{"ontologies":{{"edu-fixture":{{"version":"0.1.0","sha256":"{sha}","file":"edu-fixture.ontology.yaml","extends":["../../../etc"]}}}}}}"#
        );
        fs::write(registry.join("index.json"), malicious_index).unwrap();
        fs::write(registry.join("edu-fixture.ontology.yaml"), EDU_YAML).unwrap();

        let error = fetch(
            dir.path(),
            &registry.display().to_string(),
            &["edu-fixture".to_string()],
            false,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::MalformedOntologyId { .. }
        ));
        assert!(!dir.path().join("packs/ontologies/edu-fixture").exists());
    }

    #[test]
    fn a_failure_partway_through_a_batch_still_pins_the_ontologies_already_vendored() {
        let dir = tempfile::tempdir().unwrap();
        let registry = dir.path().join("registry");
        fs::create_dir_all(&registry).unwrap();
        let good_sha = sha256_of(EDU_YAML.as_bytes());
        let index = format!(
            r#"{{"ontologies":{{
                "edu-fixture":{{"version":"0.1.0","sha256":"{good_sha}","file":"edu-fixture.ontology.yaml","extends":[]}},
                "bad-fixture":{{"version":"0.1.0","sha256":"0000000000000000000000000000000000000000000000000000000000000000","file":"edu-fixture.ontology.yaml","extends":[]}}
            }}}}"#
        );
        fs::write(registry.join("index.json"), index).unwrap();
        fs::write(registry.join("edu-fixture.ontology.yaml"), EDU_YAML).unwrap();

        let source = registry.display().to_string();
        let error = fetch(
            dir.path(),
            &source,
            &["edu-fixture".to_string(), "bad-fixture".to_string()],
            false,
        )
        .unwrap_err();
        assert!(matches!(error, super::MifRhError::ChecksumMismatch { .. }));

        // edu-fixture was vendored successfully before bad-fixture failed;
        // it must be pinned in the lock, not left as an untracked,
        // unverifiable file on disk.
        let lock: super::LockFile = serde_json::from_str(
            &fs::read_to_string(dir.path().join("ontologies.lock.json")).unwrap(),
        )
        .unwrap();
        assert!(lock.ontologies.contains_key("edu-fixture"));
        assert!(dir.path().join("packs/ontologies/edu-fixture").is_dir());
    }

    #[test]
    fn lock_check_passes_cleanly_with_no_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("harness.config.json"),
            r#"{"ontologies":[]}"#,
        )
        .unwrap();
        let report = lock_check(dir.path(), &dir.path().join("harness.config.json")).unwrap();
        assert!(report.ok());
    }

    #[test]
    fn lock_check_flags_a_missing_pin_for_an_enabled_ontology() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("harness.config.json");
        fs::write(
            &config_path,
            r#"{"ontologies":[{"id":"edu-fixture","enabled":true}]}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("ontologies.lock.json"),
            r#"{"schema":"mif-ontology-lock/v1","source":"x","ontologies":{}}"#,
        )
        .unwrap();

        let report = lock_check(dir.path(), &config_path).unwrap();
        assert!(!report.ok());
        assert_eq!(report.missing_pins, ["edu-fixture"]);
    }

    #[test]
    fn lock_check_flags_drift_when_a_vendored_file_no_longer_matches_its_pin() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("harness.config.json");
        fs::write(&config_path, r#"{"ontologies":[]}"#).unwrap();
        fs::create_dir_all(dir.path().join("packs/ontologies/edu-fixture")).unwrap();
        fs::write(
            dir.path()
                .join("packs/ontologies/edu-fixture/edu-fixture.ontology.yaml"),
            "edited locally",
        )
        .unwrap();
        fs::write(
            dir.path().join("ontologies.lock.json"),
            r#"{"schema":"mif-ontology-lock/v1","source":"x","ontologies":{"edu-fixture":{"version":"0.1.0","sha256":"deadbeef"}}}"#,
        )
        .unwrap();

        let report = lock_check(dir.path(), &config_path).unwrap();
        assert!(!report.ok());
        assert_eq!(report.drift.len(), 1);
        assert_eq!(report.drift[0].id, "edu-fixture");
    }

    #[test]
    fn sync_catalog_catalogs_core_and_enabled_ontologies() {
        let dir = tempfile::tempdir().unwrap();
        write_base_layer(dir.path());
        let config_path = dir.path().join("harness.config.json");
        fs::write(
            &config_path,
            r#"{"ontologies":[{"id":"edu-fixture","enabled":true}]}"#,
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("packs/ontologies/edu-fixture")).unwrap();
        fs::write(
            dir.path()
                .join("packs/ontologies/edu-fixture/edu-fixture.ontology.yaml"),
            EDU_YAML,
        )
        .unwrap();
        let sidecar_path = dir.path().join("enabled-packs.json");
        fs::write(&sidecar_path, r#"{"enabledPlugins":["x"]}"#).unwrap();

        let report = sync_catalog(dir.path(), &config_path, &sidecar_path).unwrap();
        assert_eq!(report.cataloged, 2);

        let sidecar: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&sidecar_path).unwrap()).unwrap();
        // The unrelated key the general pack-sync half owns must survive.
        assert_eq!(sidecar["enabledPlugins"], serde_json::json!(["x"]));
        let ontologies = sidecar["ontologies"].as_array().unwrap();
        assert!(
            ontologies
                .iter()
                .any(|e| e["id"] == "mif-base" && e["core"] == true)
        );
        assert!(
            ontologies
                .iter()
                .any(|e| e["id"] == "edu-fixture" && e["core"] == false)
        );
    }

    #[test]
    fn sync_catalog_creates_the_sidecars_parent_directory_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("harness.config.json");
        fs::write(&config_path, r#"{"ontologies":[]}"#).unwrap();
        // No `.claude/` directory exists yet — a fresh clone before any
        // sync has ever run.
        let sidecar_path = dir.path().join(".claude/enabled-packs.json");

        let report = sync_catalog(dir.path(), &config_path, &sidecar_path).unwrap();
        assert_eq!(report.cataloged, 0);
        assert!(sidecar_path.is_file());
    }

    #[test]
    fn sync_registry_discovers_and_enables_a_new_registry_ontology_and_preserves_other_config_fields()
     {
        let dir = tempfile::tempdir().unwrap();
        write_base_layer(dir.path());
        let source = write_local_registry(dir.path());
        let config_path = dir.path().join("harness.config.json");
        fs::write(
            &config_path,
            r#"{"version":"1.2.3","topics":[{"id":"t","ontologies":[]}],"ontologies":[]}"#,
        )
        .unwrap();
        let sidecar_path = dir.path().join("enabled-packs.json");

        let report = sync_registry(dir.path(), &config_path, &sidecar_path, &source).unwrap();

        assert_eq!(report.discovered, ["edu-fixture"]);
        assert_eq!(report.fetch.vendored.len(), 1);

        let config: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&config_path).unwrap()).unwrap();
        // Untouched top-level fields must survive the read-modify-write.
        assert_eq!(config["version"], "1.2.3");
        assert_eq!(config["topics"][0]["id"], "t");
        assert_eq!(config["ontologies"][0]["id"], "edu-fixture");
        assert_eq!(config["ontologies"][0]["enabled"], true);
    }

    #[test]
    fn sync_registry_is_idempotent_when_nothing_new_is_published() {
        let dir = tempfile::tempdir().unwrap();
        write_base_layer(dir.path());
        let source = write_local_registry(dir.path());
        let config_path = dir.path().join("harness.config.json");
        fs::write(
            &config_path,
            r#"{"ontologies":[{"id":"edu-fixture","enabled":true}]}"#,
        )
        .unwrap();
        let sidecar_path = dir.path().join("enabled-packs.json");

        let report = sync_registry(dir.path(), &config_path, &sidecar_path, &source).unwrap();
        assert!(report.discovered.is_empty());
    }

    #[test]
    fn sync_registry_rejects_a_non_array_ontologies_field_without_touching_the_network() {
        // A deliberately unreachable source: if the config-shape check ran
        // AFTER fetching the index (as it once did), this would hang or
        // fail on a network error instead of failing closed immediately on
        // the malformed config, matching
        // `sync-registry-ontologies.sh`'s own original ordering (config
        // validated before any registry access).
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("harness.config.json");
        fs::write(&config_path, r#"{"ontologies":"not-an-array"}"#).unwrap();
        let sidecar_path = dir.path().join("enabled-packs.json");

        let error = sync_registry(
            dir.path(),
            &config_path,
            &sidecar_path,
            "http://127.0.0.1.invalid/unreachable",
        )
        .unwrap_err();
        assert!(matches!(error, super::MifRhError::ConfigMalformed { .. }));
    }

    #[test]
    fn sync_registry_rejects_invalid_json_config_without_touching_the_network() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("harness.config.json");
        fs::write(&config_path, "not json at all").unwrap();
        let sidecar_path = dir.path().join("enabled-packs.json");

        let error = sync_registry(
            dir.path(),
            &config_path,
            &sidecar_path,
            "http://127.0.0.1.invalid/unreachable",
        )
        .unwrap_err();
        assert!(matches!(error, super::MifRhError::Json { .. }));
    }

    /// Seeds a full pin-safety fixture: a vendored (old) pack on disk at
    /// `locked_version`, a registry (new) pack at `registry_version`
    /// (`title` entity type's `schema.required` growing from `[name]` to
    /// `[name, isbn]`), and a lock pinned at `locked_version`. Returns the
    /// registry source path.
    fn seed_pin_safety_fixture(
        dir: &std::path::Path,
        locked_version: &str,
        registry_version: &str,
    ) -> String {
        let old_yaml = format!(
            "ontology:\n  id: edu-fixture\n  version: \"{locked_version}\"\n  extends: \
             [mif-base]\nentity_types:\n  - name: title\n    schema:\n      required: \
             [name]\n      properties:\n        name: {{type: string}}\n"
        );
        fs::create_dir_all(dir.join("packs/ontologies/edu-fixture")).unwrap();
        fs::write(
            dir.join("packs/ontologies/edu-fixture/edu-fixture.ontology.yaml"),
            &old_yaml,
        )
        .unwrap();

        let registry = dir.join("registry");
        fs::create_dir_all(&registry).unwrap();
        let new_yaml = format!(
            "ontology:\n  id: edu-fixture\n  version: \"{registry_version}\"\n  extends: \
             [mif-base]\nentity_types:\n  - name: title\n    schema:\n      required: [name, \
             isbn]\n      properties:\n        name: {{type: string}}\n        isbn: {{type: \
             string}}\n"
        );
        let sha = sha256_of(new_yaml.as_bytes());
        let index = format!(
            r#"{{"ontologies":{{"edu-fixture":{{"version":"{registry_version}","sha256":"{sha}","file":"edu-fixture.ontology.yaml","extends":["mif-base"]}}}}}}"#
        );
        fs::write(registry.join("index.json"), &index).unwrap();
        fs::write(registry.join("edu-fixture.ontology.yaml"), &new_yaml).unwrap();

        let source = registry.display().to_string();
        let lock = serde_json::json!({
            "schema": "mif-ontology-lock/v1",
            "source": source,
            "index_sha256": sha256_of(index.as_bytes()),
            "ontologies": {
                "edu-fixture": {"version": locked_version, "sha256": "irrelevant-for-this-test"}
            }
        });
        fs::write(
            dir.join("ontologies.lock.json"),
            serde_json::to_string(&lock).unwrap(),
        )
        .unwrap();
        source
    }

    /// Writes one topic's `ontology-map.json` (one stamped `title` record
    /// resolved against `edu-fixture@{locked_version}`) and a matching
    /// finding file whose `entity` payload is `entity_json`.
    fn write_stamped_finding(
        reports_dir: &std::path::Path,
        topic: &str,
        locked_version: &str,
        entity_json: &str,
    ) {
        let topic_dir = reports_dir.join(topic);
        fs::create_dir_all(topic_dir.join("findings")).unwrap();
        fs::write(
            topic_dir.join("ontology-map.json"),
            format!(
                r#"[{{"finding_id":"f-1","entity_type":"title","resolved_ontology":"edu-fixture@{locked_version}","basis":"resolved","valid":true}}]"#
            ),
        )
        .unwrap();
        fs::write(
            topic_dir.join("findings/f-1.json"),
            format!(r#"{{"@id":"f-1","entity":{entity_json}}}"#),
        )
        .unwrap();
    }

    #[test]
    fn diff_newly_required_dedupes_a_field_repeated_in_the_new_schema() {
        let old_yaml = "ontology:\n  id: edu-fixture\n  version: \"0.1.0\"\n  extends: \
                        [mif-base]\nentity_types:\n  - name: title\n    schema:\n      required: \
                        [name]\n      properties:\n        name: {type: string}\n";
        let new_yaml = "ontology:\n  id: edu-fixture\n  version: \"0.2.0\"\n  extends: \
                        [mif-base]\nentity_types:\n  - name: title\n    schema:\n      required: \
                        [name, isbn, isbn]\n      properties:\n        name: {type: string}\n        \
                        isbn: {type: string}\n";
        let old_pack = parse_pack(old_yaml, "old").unwrap();
        let new_pack = parse_pack(new_yaml, "new").unwrap();

        let newly_required = diff_newly_required(&old_pack, &new_pack).unwrap();

        assert_eq!(
            newly_required.len(),
            1,
            "a required field repeated in the new schema must be reported once, not once per repetition"
        );
        assert_eq!(newly_required[0].entity_type, "title");
        assert_eq!(newly_required[0].field, "isbn");
    }

    #[test]
    fn check_pin_safety_reports_nothing_for_an_id_with_no_lock_entry() {
        let dir = tempfile::tempdir().unwrap();
        let source = seed_pin_safety_fixture(dir.path(), "0.1.0", "0.2.0");
        fs::remove_file(dir.path().join("ontologies.lock.json")).unwrap();

        let reports = super::check_pin_safety(
            dir.path(),
            &source,
            &dir.path().join("reports"),
            &[],
            &["edu-fixture".to_string()],
        )
        .unwrap();
        assert!(reports.is_empty());
    }

    #[test]
    fn check_pin_safety_reports_nothing_when_not_drifted() {
        let dir = tempfile::tempdir().unwrap();
        let source = seed_pin_safety_fixture(dir.path(), "0.2.0", "0.2.0");

        let reports = super::check_pin_safety(
            dir.path(),
            &source,
            &dir.path().join("reports"),
            &[],
            &["edu-fixture".to_string()],
        )
        .unwrap();
        assert!(reports.is_empty());
    }

    #[test]
    fn check_pin_safety_flags_a_newly_required_field_missing_from_a_stamped_finding() {
        let dir = tempfile::tempdir().unwrap();
        let source = seed_pin_safety_fixture(dir.path(), "0.1.0", "0.2.0");
        let reports_dir = dir.path().join("reports");
        write_stamped_finding(&reports_dir, "edu", "0.1.0", r#"{"name":"Algebra I"}"#);

        let reports = super::check_pin_safety(
            dir.path(),
            &source,
            &reports_dir,
            &["edu".to_string()],
            &["edu-fixture".to_string()],
        )
        .unwrap();

        assert_eq!(reports.len(), 1);
        let report = &reports[0];
        assert_eq!(report.locked_version, "0.1.0");
        assert_eq!(report.registry_version, "0.2.0");
        assert_eq!(report.newly_required.len(), 1);
        assert_eq!(report.newly_required[0].entity_type, "title");
        assert_eq!(report.newly_required[0].field, "isbn");
        assert_eq!(report.gaps.len(), 1);
        assert_eq!(report.gaps[0].finding_id, "f-1");
        assert_eq!(report.gaps[0].topic, "edu");
        assert_eq!(report.gaps[0].field, "isbn");
    }

    #[test]
    fn check_pin_safety_reports_no_gap_when_the_stamped_finding_already_has_the_field() {
        let dir = tempfile::tempdir().unwrap();
        let source = seed_pin_safety_fixture(dir.path(), "0.1.0", "0.2.0");
        let reports_dir = dir.path().join("reports");
        write_stamped_finding(
            &reports_dir,
            "edu",
            "0.1.0",
            r#"{"name":"Algebra I","isbn":"978-0-13-000000-0"}"#,
        );

        let reports = super::check_pin_safety(
            dir.path(),
            &source,
            &reports_dir,
            &["edu".to_string()],
            &["edu-fixture".to_string()],
        )
        .unwrap();

        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].newly_required.len(), 1);
        assert!(
            reports[0].gaps.is_empty(),
            "a finding that already carries the newly required field must not be flagged"
        );
    }

    #[test]
    fn check_pin_safety_reports_no_extra_warning_when_nothing_newly_required() {
        let dir = tempfile::tempdir().unwrap();
        // Same required list on both sides: no newly required field.
        let old_yaml = "ontology:\n  id: edu-fixture\n  version: \"0.1.0\"\n  extends: \
                         [mif-base]\nentity_types:\n  - name: title\n    schema:\n      \
                         required: [name]\n";
        fs::create_dir_all(dir.path().join("packs/ontologies/edu-fixture")).unwrap();
        fs::write(
            dir.path()
                .join("packs/ontologies/edu-fixture/edu-fixture.ontology.yaml"),
            old_yaml,
        )
        .unwrap();
        let registry = dir.path().join("registry");
        fs::create_dir_all(&registry).unwrap();
        let new_yaml = "ontology:\n  id: edu-fixture\n  version: \"0.2.0\"\n  extends: \
                         [mif-base]\nentity_types:\n  - name: title\n    schema:\n      \
                         required: [name]\n";
        let sha = sha256_of(new_yaml.as_bytes());
        let index = format!(
            r#"{{"ontologies":{{"edu-fixture":{{"version":"0.2.0","sha256":"{sha}","file":"edu-fixture.ontology.yaml","extends":["mif-base"]}}}}}}"#
        );
        fs::write(registry.join("index.json"), &index).unwrap();
        fs::write(registry.join("edu-fixture.ontology.yaml"), new_yaml).unwrap();
        let source = registry.display().to_string();
        let lock = serde_json::json!({
            "schema": "mif-ontology-lock/v1",
            "source": source,
            "index_sha256": sha256_of(index.as_bytes()),
            "ontologies": {"edu-fixture": {"version": "0.1.0", "sha256": "irrelevant"}}
        });
        fs::write(
            dir.path().join("ontologies.lock.json"),
            serde_json::to_string(&lock).unwrap(),
        )
        .unwrap();

        let reports = super::check_pin_safety(
            dir.path(),
            &source,
            &dir.path().join("reports"),
            &[],
            &["edu-fixture".to_string()],
        )
        .unwrap();

        assert_eq!(reports.len(), 1);
        assert!(reports[0].newly_required.is_empty());
        assert!(reports[0].gaps.is_empty());
    }

    #[test]
    fn check_pin_safety_reports_empty_analysis_when_the_vendored_pack_is_missing_from_disk() {
        let dir = tempfile::tempdir().unwrap();
        let source = seed_pin_safety_fixture(dir.path(), "0.1.0", "0.2.0");
        fs::remove_file(
            dir.path()
                .join("packs/ontologies/edu-fixture/edu-fixture.ontology.yaml"),
        )
        .unwrap();

        let reports = super::check_pin_safety(
            dir.path(),
            &source,
            &dir.path().join("reports"),
            &[],
            &["edu-fixture".to_string()],
        )
        .unwrap();

        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].locked_version, "0.1.0");
        assert_eq!(reports[0].registry_version, "0.2.0");
        assert!(!reports[0].analyzed);
        assert!(reports[0].newly_required.is_empty());
        assert!(reports[0].gaps.is_empty());
    }

    #[test]
    fn check_pin_safety_reports_an_unrelated_ontology_id_error_for_an_unregistered_id() {
        let dir = tempfile::tempdir().unwrap();
        let source = seed_pin_safety_fixture(dir.path(), "0.1.0", "0.2.0");
        // Pin an id the registry doesn't know about.
        let mut lock: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.path().join("ontologies.lock.json")).unwrap(),
        )
        .unwrap();
        lock["ontologies"]["ghost-ontology"] =
            serde_json::json!({"version": "0.1.0", "sha256": "irrelevant"});
        fs::write(
            dir.path().join("ontologies.lock.json"),
            serde_json::to_string(&lock).unwrap(),
        )
        .unwrap();

        let error = super::check_pin_safety(
            dir.path(),
            &source,
            &dir.path().join("reports"),
            &[],
            &["ghost-ontology".to_string()],
        )
        .unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::OntologyNotInRegistry { .. }
        ));
    }

    #[test]
    fn check_pin_safety_fails_closed_on_a_checksum_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let source = seed_pin_safety_fixture(dir.path(), "0.1.0", "0.2.0");
        // Corrupt the registry's declared sha256 for the drifted id's file.
        let index_path = dir.path().join("registry/index.json");
        let mut index: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&index_path).unwrap()).unwrap();
        index["ontologies"]["edu-fixture"]["sha256"] =
            serde_json::json!("0000000000000000000000000000000000000000000000000000000000000000");
        fs::write(&index_path, serde_json::to_string(&index).unwrap()).unwrap();
        // Re-write the lock's index_sha256 to match the corrupted index, so
        // this test isolates the file-checksum check from the index-pin
        // check.
        let lock_path = dir.path().join("ontologies.lock.json");
        let mut lock: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&lock_path).unwrap()).unwrap();
        lock["index_sha256"] =
            serde_json::json!(sha256_of(fs::read(&index_path).unwrap().as_slice()));
        fs::write(&lock_path, serde_json::to_string(&lock).unwrap()).unwrap();

        let error = super::check_pin_safety(
            dir.path(),
            &source,
            &dir.path().join("reports"),
            &[],
            &["edu-fixture".to_string()],
        )
        .unwrap_err();
        assert!(matches!(error, super::MifRhError::ChecksumMismatch { .. }));
    }

    #[test]
    fn check_pin_safety_rejects_an_unsafe_index_path() {
        let dir = tempfile::tempdir().unwrap();
        let source = seed_pin_safety_fixture(dir.path(), "0.1.0", "0.2.0");
        let index_path = dir.path().join("registry/index.json");
        let mut index: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&index_path).unwrap()).unwrap();
        index["ontologies"]["edu-fixture"]["file"] = serde_json::json!("../../../../etc/passwd");
        fs::write(&index_path, serde_json::to_string(&index).unwrap()).unwrap();
        let lock_path = dir.path().join("ontologies.lock.json");
        let mut lock: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&lock_path).unwrap()).unwrap();
        lock["index_sha256"] =
            serde_json::json!(sha256_of(fs::read(&index_path).unwrap().as_slice()));
        fs::write(&lock_path, serde_json::to_string(&lock).unwrap()).unwrap();

        let error = super::check_pin_safety(
            dir.path(),
            &source,
            &dir.path().join("reports"),
            &[],
            &["edu-fixture".to_string()],
        )
        .unwrap_err();
        assert!(matches!(error, super::MifRhError::UnsafeIndexPath { .. }));
    }

    #[test]
    fn check_pin_safety_refuses_a_registry_index_that_no_longer_matches_the_pinned_trust_root() {
        let dir = tempfile::tempdir().unwrap();
        let source = seed_pin_safety_fixture(dir.path(), "0.1.0", "0.2.0");
        // seed_pin_safety_fixture already pins the real index sha256;
        // corrupt it to simulate a swapped/tampered registry source.
        let lock_path = dir.path().join("ontologies.lock.json");
        let mut lock: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&lock_path).unwrap()).unwrap();
        lock["index_sha256"] = serde_json::json!("not-the-real-index-sha256");
        fs::write(&lock_path, serde_json::to_string(&lock).unwrap()).unwrap();

        let error = super::check_pin_safety(
            dir.path(),
            &source,
            &dir.path().join("reports"),
            &[],
            &["edu-fixture".to_string()],
        )
        .unwrap_err();
        assert!(matches!(error, super::MifRhError::IndexPinMismatch { .. }));
    }

    #[test]
    fn check_pin_safety_rejects_a_malformed_id_before_touching_the_filesystem() {
        let dir = tempfile::tempdir().unwrap();
        let source = seed_pin_safety_fixture(dir.path(), "0.1.0", "0.2.0");

        let error = super::check_pin_safety(
            dir.path(),
            &source,
            &dir.path().join("reports"),
            &[],
            &["../../../../etc/passwd".to_string()],
        )
        .unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::MalformedOntologyId { .. }
        ));
    }

    #[test]
    fn check_pin_safety_fails_closed_on_a_malformed_schema_required_field() {
        let dir = tempfile::tempdir().unwrap();
        let old_yaml = "ontology:\n  id: edu-fixture\n  version: \"0.1.0\"\n  extends: \
                         [mif-base]\nentity_types:\n  - name: title\n    schema:\n      \
                         required: [name]\n      properties:\n        name: {type: string}\n";
        fs::create_dir_all(dir.path().join("packs/ontologies/edu-fixture")).unwrap();
        fs::write(
            dir.path()
                .join("packs/ontologies/edu-fixture/edu-fixture.ontology.yaml"),
            old_yaml,
        )
        .unwrap();

        // `required` is present but is a string, not an array: malformed.
        let registry = dir.path().join("registry");
        fs::create_dir_all(&registry).unwrap();
        let new_yaml = "ontology:\n  id: edu-fixture\n  version: \"0.2.0\"\n  extends: \
                         [mif-base]\nentity_types:\n  - name: title\n    schema:\n      \
                         required: \"name\"\n      properties:\n        name: {type: string}\n";
        let sha = sha256_of(new_yaml.as_bytes());
        let index = format!(
            r#"{{"ontologies":{{"edu-fixture":{{"version":"0.2.0","sha256":"{sha}","file":"edu-fixture.ontology.yaml","extends":["mif-base"]}}}}}}"#
        );
        fs::write(registry.join("index.json"), &index).unwrap();
        fs::write(registry.join("edu-fixture.ontology.yaml"), new_yaml).unwrap();

        let source = registry.display().to_string();
        let lock = serde_json::json!({
            "schema": "mif-ontology-lock/v1",
            "source": source,
            "index_sha256": sha256_of(index.as_bytes()),
            "ontologies": {
                "edu-fixture": {"version": "0.1.0", "sha256": "irrelevant-for-this-test"}
            }
        });
        fs::write(
            dir.path().join("ontologies.lock.json"),
            serde_json::to_string(&lock).unwrap(),
        )
        .unwrap();

        let error = super::check_pin_safety(
            dir.path(),
            &source,
            &dir.path().join("reports"),
            &[],
            &["edu-fixture".to_string()],
        )
        .unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::EntityTypeSchemaInvalid { .. }
        ));
    }
}
