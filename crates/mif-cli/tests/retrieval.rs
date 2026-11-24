//! Proves a document is found by text that sits far past the embedding
//! model's input window, against the compiled binary.
//!
//! Every document in the corpus opens with the same long boilerplate, the
//! shape that made one-vector-per-document search rank templated records
//! almost at random: the first 512 tokens of each body were identical, so
//! each document's single vector said nothing about what set it apart. The
//! text that does set each document apart comes after the boilerplate.

use std::process::Command;

/// The distinguishing sentence of each document, with the query that should
/// find it. Queries paraphrase; they share few words with their sentence.
const SUBJECTS: [(&str, &str, &str); 5] = [
    (
        "cats",
        "Cats are small domesticated felines kept as house pets.",
        "a furry pet that purrs and chases mice",
    ),
    (
        "finance",
        "Quarterly revenue exceeded what analysts had forecast.",
        "the company earned more money than expected this quarter",
    ),
    (
        "volcano",
        "The volcano erupted and sent lava down the mountainside.",
        "molten rock flowing from an eruption",
    ),
    (
        "bread",
        "Knead the dough, let it rise, then bake the loaf until golden.",
        "how to make bread in an oven",
    ),
    (
        "orbit",
        "The satellite circles the planet once every ninety minutes.",
        "a spacecraft in low orbit around the earth",
    ),
];

/// Well over the model's 512-token window, and the same in every document.
fn boilerplate() -> String {
    "This record follows the standard template. Read the purpose, the owner, and the review \
     cadence before relying on anything below. Changes are proposed by the practice and \
     confirmed by the named role. "
        .repeat(30)
}

#[test]
fn a_document_is_found_by_text_past_the_model_window() {
    let mif_cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_mif-cli"))
            .args(args)
            .output()
            .unwrap()
    };
    let dir = tempfile::tempdir().unwrap();
    let docs = dir.path().join("docs");
    std::fs::create_dir(&docs).unwrap();
    for (name, sentence, _query) in SUBJECTS {
        let body = format!(
            "---\nid: {}\ntype: semantic\ncreated: 2026-07-02T00:00:00Z\ntitle: Record {name}\n---\n\n\
             # Record {name}\n\n## How to use this\n\n{}\n\n## Detail\n\n{sentence}\n",
            mif_core::concept_uuid(name),
            boilerplate()
        );
        std::fs::write(docs.join(format!("{name}.md")), body).unwrap();
    }
    let db = dir.path().join("vectors.db");
    let db = db.to_str().unwrap();

    let ingest = mif_cli(&["ingest", docs.to_str().unwrap(), "--db-path", db]);
    let stderr = String::from_utf8_lossy(&ingest.stderr);
    // The same conditions the unit tests skip on: the model could not be
    // fetched or cached on this machine.
    let model_unavailable = [
        "failed to fetch",
        "no user cache directory",
        "Hugging Face Hub client",
    ]
    .iter()
    .any(|reason| stderr.contains(reason));
    if !ingest.status.success() && model_unavailable {
        eprintln!("skipping: could not load embedding model: {stderr}");
        return;
    }
    assert!(ingest.status.success(), "ingest failed: {stderr}");
    let stdout = String::from_utf8_lossy(&ingest.stdout);
    assert!(stdout.contains("ingested 5 documents"), "{stdout}");

    for (name, _sentence, query) in SUBJECTS {
        let search = mif_cli(&[
            "search",
            query,
            "--db-path",
            db,
            "--limit",
            "1",
            "--show-passage",
        ]);
        assert!(search.status.success());
        let top = String::from_utf8_lossy(&search.stdout);
        let expected = format!("urn:mif:{}  [Detail]", mif_core::concept_uuid(name));
        assert!(
            top.trim_end().ends_with(&expected),
            "query {query:?} should find {name} by its Detail section, got: {top}"
        );
    }
}
