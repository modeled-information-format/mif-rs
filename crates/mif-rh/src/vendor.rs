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

use std::collections::{BTreeMap, HashSet};
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

/// The outcome of a [`fetch`] call.
#[derive(Debug, Clone, Default)]
pub struct FetchReport {
    /// Every ontology newly vendored by this call (already-vendored,
    /// unchanged ontologies are not re-listed).
    pub vendored: Vec<VendoredOntology>,
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
pub fn fetch(root: &Path, source: &str, ids: &[String]) -> Result<FetchReport, MifRhError> {
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
    let packs_dir = root.join("packs/ontologies");
    for id in &fetch_list {
        // Present in `index` by construction: `resolve_fetch_set` already
        // resolved every entry in `fetch_list` through a successful lookup.
        let Some(entry) = index.ontologies.get(id) else {
            continue;
        };
        if entry.file.contains('/') || entry.file.contains("..") {
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
        std::fs::create_dir_all(&dest_dir).map_err(|source| MifRhError::Io {
            path: dest_dir.display().to_string(),
            source,
        })?;
        let out_yaml = dest_dir.join(format!("{id}.ontology.yaml"));
        std::fs::write(&out_yaml, &bytes).map_err(|source| MifRhError::Io {
            path: out_yaml.display().to_string(),
            source,
        })?;

        let pack = parse_pack(
            &String::from_utf8_lossy(&bytes),
            &out_yaml.display().to_string(),
        )?;
        let sidecar = serde_json::json!({
            "name": id,
            "version": entry.version,
            "kind": "ontology",
            "description": pack.description,
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
        vendored.push(VendoredOntology {
            id: id.clone(),
            version: entry.version.clone(),
        });
    }

    lock.index_sha256 = Some(index_sha256);
    lock.source = source.to_string();
    crate::write_json_atomic(&lock_path, &lock)?;

    Ok(FetchReport { vendored })
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
    let index_bytes = fetch_raw(source, "index.json")?;
    let index: RegistryIndex =
        serde_json::from_slice(&index_bytes).map_err(|err| MifRhError::RegistryIndexInvalid {
            registry_source: source.to_string(),
            detail: err.to_string(),
        })?;

    let mut config = load_config_json(config_path)?;
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
    let fetch_report = fetch(root, source, &enabled)?;
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

    for id in enabled_ontology_ids(&config) {
        if is_committed_base(root, &id) {
            continue;
        }
        if !lock.ontologies.contains_key(&id) {
            report.missing_pins.push(id);
        }
    }

    for (id, entry) in &lock.ontologies {
        let yaml = root
            .join("packs/ontologies")
            .join(id)
            .join(format!("{id}.ontology.yaml"));
        if !yaml.is_file() {
            if enabled_ontology_ids(&config).contains(id) {
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

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        DEFAULT_REGISTRY_SOURCE, fetch, is_wellformed_id, lock_check, resolve_source, sync_catalog,
        sync_registry,
    };

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

        let report = fetch(dir.path(), &source, &["edu-fixture".to_string()]).unwrap();

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
        )
        .unwrap_err();
        assert!(matches!(error, super::MifRhError::ChecksumMismatch { .. }));
        assert!(!dir.path().join("packs/ontologies/edu-fixture").exists());
    }

    #[test]
    fn fetch_refuses_a_moved_trust_root_for_the_same_source() {
        let dir = tempfile::tempdir().unwrap();
        write_base_layer(dir.path());
        let source = write_local_registry(dir.path());
        fetch(dir.path(), &source, &["edu-fixture".to_string()]).unwrap();

        // The same source's index.json now hashes differently.
        fs::write(
            std::path::Path::new(&source).join("index.json"),
            EDU_INDEX
                .replace("REPLACED", &sha256_of(EDU_YAML.as_bytes()))
                .replace('0', "1"),
        )
        .unwrap();

        let error = fetch(dir.path(), &source, &["edu-fixture".to_string()]).unwrap_err();
        assert!(matches!(error, super::MifRhError::IndexPinMismatch { .. }));
    }

    #[test]
    fn fetch_reports_an_id_absent_from_the_registry() {
        let dir = tempfile::tempdir().unwrap();
        let source = write_local_registry(dir.path());

        let error = fetch(dir.path(), &source, &["nonexistent-id".to_string()]).unwrap_err();
        assert!(matches!(
            error,
            super::MifRhError::OntologyNotInRegistry { .. }
        ));
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
}
