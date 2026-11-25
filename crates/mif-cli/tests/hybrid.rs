//! Hybrid search, the JSON result payload, the type filters, and
//! `get-passage`, against the compiled binary.

// Helpers outside a `#[test]` function are not covered by this workspace's
// allow-unwrap-in-tests setting.
#![allow(clippy::unwrap_used, clippy::print_stderr)]

use std::path::Path;
use std::process::{Command, Output};

fn mif_cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_mif-cli"))
        .args(args)
        .output()
        .unwrap()
}

/// The same conditions the unit tests skip on: the model could not be
/// fetched or cached on this machine.
fn model_unavailable(output: &Output) -> bool {
    let stderr = String::from_utf8_lossy(&output.stderr);
    !output.status.success()
        && [
            "failed to fetch",
            "no user cache directory",
            "Hugging Face Hub client",
        ]
        .iter()
        .any(|reason| stderr.contains(reason))
}

fn document(name: &str, title: &str, entity_type: Option<&str>, body: &str) -> String {
    let entity = entity_type.map_or_else(String::new, |entity_type| {
        format!("entity:\n  name: {title}\n  entity_type: {entity_type}\n")
    });
    format!(
        "---\nid: {}\ntype: semantic\ncreated: 2026-07-02T00:00:00Z\nnamespace: records\n\
         title: {title}\n{entity}---\n\n{body}\n",
        mif_core::concept_uuid(name)
    )
}

fn urn(name: &str) -> String {
    format!("urn:mif:{}", mif_core::concept_uuid(name))
}

/// Writes the corpus and ingests it. `None` when the model is unavailable.
fn corpus(dir: &Path) -> Option<String> {
    let docs = dir.join("docs");
    std::fs::create_dir(&docs).unwrap();
    let write = |name: &str, contents: String| {
        std::fs::write(docs.join(format!("{name}.md")), contents).unwrap();
    };
    write(
        "consent",
        document(
            "consent",
            "Consent handling",
            Some("practice"),
            "# Consent handling\n\nHow consent is recorded.\n\n## Scope\n\n\
             Practice P23 applies to every [intake form](forms/intake.md).\n\n\
             ## Notes\n\nFirst note.\n\n## Owner\n\nThe privacy office.\n\n## Notes\n\nSecond note.",
        ),
    );
    write(
        "privacy",
        document(
            "privacy",
            "Privacy and personal data",
            Some("policy"),
            "# Privacy and personal data\n\n## Scope\n\n\
             Rules for collecting, keeping and deleting information about people.",
        ),
    );
    write(
        "bread",
        document(
            "bread",
            "Baking bread",
            None,
            "A weekend routine.\n\n## Method\n\nKnead the dough, let it rise, then bake the loaf.",
        ),
    );
    // The store sits in `.mif` beside the documents, so each recorded
    // path is relative to `dir`.
    std::fs::create_dir(dir.join(".mif")).unwrap();
    let db = dir
        .join(".mif")
        .join("vectors.db")
        .to_string_lossy()
        .into_owned();
    let ingest = mif_cli(&["ingest", docs.to_str().unwrap(), "--db-path", &db]);
    if model_unavailable(&ingest) {
        eprintln!("skipping: could not load embedding model");
        return None;
    }
    assert!(
        ingest.status.success(),
        "ingest failed: {}",
        String::from_utf8_lossy(&ingest.stderr)
    );
    Some(db)
}

fn search_json(db: &str, args: &[&str]) -> Vec<serde_json::Value> {
    let mut all = vec!["--format", "json", "search", "--db-path", db];
    all.extend(args);
    let output = mif_cli(&all);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let parsed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    parsed["matches"].as_array().unwrap().clone()
}

fn ids(matches: &[serde_json::Value]) -> Vec<&str> {
    matches.iter().map(|m| m["id"].as_str().unwrap()).collect()
}

#[test]
fn an_identifier_is_found_by_the_passage_that_holds_it() {
    let dir = tempfile::tempdir().unwrap();
    let Some(db) = corpus(dir.path()) else {
        return;
    };

    let matches = search_json(&db, &["P23"]);

    let top = &matches[0];
    assert_eq!(top["id"], urn("consent"));
    assert_eq!(top["path"], "docs/consent.md");
    assert_eq!(top["title"], "Consent handling");
    assert_eq!(top["entity_type"], "practice");
    assert_eq!(top["concept_type"], "semantic");
    assert_eq!(top["heading"], "Scope");
    assert_eq!(top["text"], "Practice P23 applies to every intake form.");
    assert!(top["bm25"].as_f64().unwrap() < 0.0);
    assert!(top["cosine"].as_f64().unwrap() <= 1.0);
    // The score is a reciprocal rank fusion value, not a cosine.
    assert!(top["score"].as_f64().unwrap() <= 2.0 / 61.0 + 1e-6);

    // The span is where the passage sits in the document body.
    let body = "# Consent handling\n\nHow consent is recorded.\n\n## Scope\n\n\
                Practice P23 applies to every [intake form](forms/intake.md).";
    let span = &top["span"];
    let start = usize::try_from(span["start_char"].as_u64().unwrap()).unwrap();
    let end = usize::try_from(span["end_char"].as_u64().unwrap()).unwrap();
    let sliced: String = body.chars().skip(start).take(end - start).collect();
    assert_eq!(
        sliced,
        "Practice P23 applies to every [intake form](forms/intake.md)."
    );
    assert_eq!(
        (span["start_line"].as_u64(), span["end_line"].as_u64()),
        (Some(7), Some(7))
    );

    // Documents that hold no query term are still ranked, by their vectors.
    assert_eq!(matches.len(), 3);
    assert!(matches[1]["bm25"].is_null());
}

#[test]
fn semantic_mode_scores_by_cosine_and_text_output_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let Some(db) = corpus(dir.path()) else {
        return;
    };

    let matches = search_json(
        &db,
        &["how to make a loaf in an oven", "--mode", "semantic"],
    );
    assert_eq!(matches[0]["id"], urn("bread"));
    assert_eq!(matches[0]["score"], matches[0]["cosine"]);
    assert!(matches.iter().all(|m| m["bm25"].is_null()));
    let scores: Vec<f64> = matches
        .iter()
        .map(|m| m["score"].as_f64().unwrap())
        .collect();
    assert!(scores.windows(2).all(|pair| pair[0] >= pair[1]));

    // Without `--format json`, a result is one `score  id` line, as before.
    let text = mif_cli(&[
        "search",
        "how to make a loaf in an oven",
        "--db-path",
        &db,
        "--limit",
        "1",
        "--show-passage",
    ]);
    let line = String::from_utf8_lossy(&text.stdout);
    assert!(
        line.trim_end()
            .ends_with(&format!("{}  [Method]", urn("bread"))),
        "{line}"
    );
    // An error format chosen for a pipe does not turn results into JSON.
    assert!(!line.trim_start().starts_with('{'));
}

#[test]
fn type_filters_take_exact_repeatable_values() {
    let dir = tempfile::tempdir().unwrap();
    let Some(db) = corpus(dir.path()) else {
        return;
    };
    let sorted = |args: &[&str]| {
        let mut all = vec!["personal information"];
        all.extend(args);
        let mut found: Vec<String> = ids(&search_json(&db, &all))
            .into_iter()
            .map(ToString::to_string)
            .collect();
        found.sort();
        found
    };
    let expect = |names: &[&str]| {
        let mut urns: Vec<String> = names.iter().map(|name| urn(name)).collect();
        urns.sort();
        urns
    };

    assert_eq!(sorted(&["--entity-type", "policy"]), expect(&["privacy"]));
    assert_eq!(
        sorted(&["--entity-type", "policy", "--entity-type", "practice"]),
        expect(&["consent", "privacy"])
    );
    assert_eq!(sorted(&["--entity-type", "pol"]), expect(&[]));
    // A document with no entity type is kept by an exclusion.
    assert_eq!(
        sorted(&["--exclude-entity-type", "policy"]),
        expect(&["bread", "consent"])
    );
    assert_eq!(
        sorted(&["--concept-type", "semantic"]),
        expect(&["bread", "consent", "privacy"])
    );
    assert_eq!(sorted(&["--exclude-concept-type", "semantic"]), expect(&[]));
}

#[test]
fn get_passage_prints_a_section_as_it_was_written() {
    let dir = tempfile::tempdir().unwrap();
    let Some(db) = corpus(dir.path()) else {
        return;
    };
    let consent = urn("consent");
    let run = |args: &[&str]| {
        let mut all = vec![
            "--format",
            "json",
            "get-passage",
            consent.as_str(),
            "--db-path",
            &db,
        ];
        all.extend(args);
        mif_cli(&all)
    };

    let scope = run(&["--heading", "Scope"]);
    let parsed: serde_json::Value = serde_json::from_slice(&scope.stdout).unwrap();
    assert_eq!(parsed["id"], consent);
    let sections = parsed["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 1);
    assert_eq!(
        sections[0]["text"],
        "## Scope\n\nPractice P23 applies to every [intake form](forms/intake.md)."
    );
    assert_eq!(sections[0]["span"]["start_line"], 5);

    // A heading that occurs twice gives both sections, in order.
    let notes = run(&["--heading", "Notes"]);
    let parsed: serde_json::Value = serde_json::from_slice(&notes.stdout).unwrap();
    let texts: Vec<&str> = parsed["sections"]
        .as_array()
        .unwrap()
        .iter()
        .map(|section| section["text"].as_str().unwrap())
        .collect();
    assert_eq!(
        texts,
        vec!["## Notes\n\nFirst note.", "## Notes\n\nSecond note."]
    );

    // No heading gives the summary.
    let summary = run(&[]);
    let parsed: serde_json::Value = serde_json::from_slice(&summary.stdout).unwrap();
    assert!(parsed["sections"][0]["heading"].is_null());
    assert!(
        parsed["sections"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Consent handling")
    );

    // A match with no heading is the summary or the text before the first
    // heading; asking with no heading gives both, and only the second has
    // a span.
    let bread = urn("bread");
    let unheaded = mif_cli(&["--format", "json", "get-passage", &bread, "--db-path", &db]);
    let parsed: serde_json::Value = serde_json::from_slice(&unheaded.stdout).unwrap();
    let sections = parsed["sections"].as_array().unwrap();
    assert_eq!(sections.len(), 2);
    assert!(sections[0]["span"].is_null());
    assert_eq!(sections[1]["text"], "A weekend routine.");
    assert!(sections[1]["heading"].is_null());
    assert_eq!(sections[1]["span"]["start_line"], 1);

    // As text, the section alone is printed.
    let text = mif_cli(&[
        "get-passage",
        &consent,
        "--heading",
        "Owner",
        "--db-path",
        &db,
    ]);
    assert_eq!(
        String::from_utf8_lossy(&text.stdout),
        "## Owner\n\nThe privacy office.\n"
    );

    let unknown_heading = run(&["--heading", "Absent"]);
    assert_eq!(unknown_heading.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&unknown_heading.stderr).contains("mif-cli-section-not-found"));
    let unknown_id = mif_cli(&[
        "--format",
        "json",
        "get-passage",
        "urn:mif:absent",
        "--db-path",
        &db,
    ]);
    assert_eq!(unknown_id.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&unknown_id.stderr).contains("mif-cli-document-not-found"));
}

#[test]
fn ingest_leaves_out_what_it_is_told_to_ignore() {
    let dir = tempfile::tempdir().unwrap();
    let docs = dir.path().join("docs");
    std::fs::create_dir_all(docs.join("archive")).unwrap();
    std::fs::write(
        docs.join("keep.md"),
        document("keep", "Keep", None, "Kept."),
    )
    .unwrap();
    std::fs::write(
        docs.join("archive").join("old.md"),
        document("old", "Old", None, "Archived."),
    )
    .unwrap();
    std::fs::write(
        docs.join("wip.draft.md"),
        document("wip", "Draft", None, "Unfinished."),
    )
    .unwrap();
    let db = dir.path().join("vectors.db");

    let ingest = mif_cli(&[
        "ingest",
        docs.to_str().unwrap(),
        "--db-path",
        db.to_str().unwrap(),
        "--ignore",
        "archive/**",
        "--ignore",
        "*.draft.md",
    ]);
    if model_unavailable(&ingest) {
        eprintln!("skipping: could not load embedding model");
        return;
    }

    let stdout = String::from_utf8_lossy(&ingest.stdout);
    assert!(stdout.contains("ingested 1 documents"), "{stdout}");
    assert!(stdout.contains(&urn("keep")));

    let bad = mif_cli(&["ingest", docs.to_str().unwrap(), "--ignore", "docs/["]);
    assert_eq!(bad.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("invalid --ignore pattern"));
}
