//! Verifies `main()`'s actual process exit code end-to-end, against the
//! compiled binary. Unit tests inside `main.rs` can prove
//! `CliError::to_problem().exit_code` maps correctly, but only a real
//! subprocess invocation proves `main()` itself carries that value through
//! `std::process::ExitCode` — this is the exact behavior a prior review
//! flagged as always returning `ExitCode::FAILURE` (1) regardless of the
//! error's mapped code.

use std::process::Command;

#[test]
fn missing_file_exits_with_the_io_error_code() {
    let output = Command::new(env!("CARGO_BIN_EXE_mif-cli"))
        .args(["validate", "/nonexistent/mif-cli-fixture.json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn invalid_json_exits_with_the_mapped_error_code() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), "not valid json").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mif-cli"))
        .args(["validate", file.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn a_conformant_document_exits_zero() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        r#"{
            "@context": "https://mif-spec.dev/schema/context.jsonld",
            "@type": "Concept",
            "@id": "urn:mif:memory:exit-code-test",
            "conceptType": "semantic",
            "content": "Content.",
            "created": "2026-07-02T00:00:00Z"
        }"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mif-cli"))
        .args(["validate", file.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn roundtrip_on_a_lossless_document_exits_zero() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        "---\nid: memory:roundtrip-cmd-test\ntype: semantic\ncreated: 2026-07-02T00:00:00Z\n---\n\nBody.\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mif-cli"))
        .args(["roundtrip", file.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("roundtrip lossless"),
        "stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[test]
fn roundtrip_on_a_drifting_document_exits_with_the_mapped_error_code() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        "---\nid: x\ntype: semantic\n123: orphaned-value\n---\n\nBody.\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mif-cli"))
        .args(["roundtrip", file.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
}

#[test]
fn emit_jsonld_prints_the_projection_and_exits_zero() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        "---\nid: memory:emit-jsonld-cmd-test\ntype: semantic\ncreated: 2026-07-02T00:00:00Z\n---\n\nBody.\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mif-cli"))
        .args(["emit-jsonld", file.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["@id"], "urn:mif:memory:emit-jsonld-cmd-test");
}

#[test]
fn emit_jsonld_with_out_writes_the_file_and_exits_zero() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        "---\nid: memory:emit-jsonld-out-test\ntype: semantic\ncreated: 2026-07-02T00:00:00Z\n---\n\nBody.\n",
    )
    .unwrap();
    let out_dir = tempfile::tempdir().unwrap();
    let out_path = out_dir.path().join("out.json");

    let output = Command::new(env!("CARGO_BIN_EXE_mif-cli"))
        .args([
            "emit-jsonld",
            file.path().to_str().unwrap(),
            "--out",
            out_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    let written = std::fs::read_to_string(&out_path).unwrap();
    let value: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(value["@id"], "urn:mif:memory:emit-jsonld-out-test");
}

#[test]
fn emit_markdown_prints_the_projection_and_exits_zero() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        r#"{
            "@context": "https://mif-spec.dev/schema/context.jsonld",
            "@type": "Concept",
            "@id": "urn:mif:memory:emit-markdown-cmd-test",
            "conceptType": "semantic",
            "content": "Content.",
            "created": "2026-07-02T00:00:00Z"
        }"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mif-cli"))
        .args(["emit-markdown", file.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("---\n"));
}

#[test]
fn emit_markdown_on_invalid_json_exits_with_the_mapped_error_code() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), "not valid json").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mif-cli"))
        .args(["emit-markdown", file.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn a_document_missing_a_level_floor_field_exits_with_the_mapped_error_code() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        r#"{
            "@context": "https://mif-spec.dev/schema/context.jsonld",
            "@type": "Concept",
            "@id": "urn:mif:memory:exit-code-level-test",
            "conceptType": "semantic",
            "content": "Content.",
            "created": "2026-07-02T00:00:00Z"
        }"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mif-cli"))
        .args(["validate", file.path().to_str().unwrap(), "--level", "2"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(5));
}

#[test]
fn an_out_of_range_level_exits_with_the_mapped_error_code() {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        file.path(),
        r#"{
            "@context": "https://mif-spec.dev/schema/context.jsonld",
            "@type": "Concept",
            "@id": "urn:mif:memory:exit-code-level-range-test",
            "conceptType": "semantic",
            "content": "Content.",
            "created": "2026-07-02T00:00:00Z"
        }"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mif-cli"))
        .args(["validate", file.path().to_str().unwrap(), "--level", "9"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
}

/// Regression test (mif-rs#69's bug class): `search`'s `QUERY` positional
/// is free-authored prose that can legitimately start with `-`. Without
/// `allow_hyphen_values`, clap misparses it as an unrecognized flag.
/// A full success run needs a downloaded embedding model, so this proves
/// the narrower thing that matters: clap accepts the value and the run
/// reaches vector-store resolution (an unrelated, expected failure in an
/// empty temp dir with no `.mif/` directory yet) instead of failing at
/// argument parsing.
#[test]
fn search_accepts_a_query_starting_with_a_hyphen() {
    let dir = tempfile::tempdir().unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mif-cli"))
        .current_dir(dir.path())
        .args(["search", "-foo bar"])
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        !stderr.contains("unexpected argument") && !stderr.contains("Usage: mif-cli search"),
        "clap must not misparse the leading-hyphen query as a flag: {stderr}"
    );
    assert!(
        stderr.contains("missing-parent-dir"),
        "expected the query to reach vector-store resolution, got: {stderr}"
    );
}
