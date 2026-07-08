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
