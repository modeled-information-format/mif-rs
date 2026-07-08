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
