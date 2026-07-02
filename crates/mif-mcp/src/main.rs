//! MCP server for the MIF (Modeled Information Format) ecosystem.
//!
//! Exposes the same two operations as `mif-cli`, as MCP tools:
//! `validate_mif_document` and `resolve_ontology_reference`. Both are thin
//! wrappers calling the identical `mif-schema`/`mif-ontology` functions
//! `mif-cli` calls — kept deliberately in lockstep rather than diverging.

use std::path::PathBuf;

use rmcp::handler::server::wrapper::Parameters;
use rmcp::transport::stdio;
use rmcp::{ServerHandler, ServiceExt, schemars, tool, tool_handler, tool_router};

/// Parameters for the `validate_mif_document` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ValidateParams {
    /// Path to the MIF document (JSON-LD projection) to validate.
    file: PathBuf,
}

/// Parameters for the `resolve_ontology_reference` tool.
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ResolveParams {
    /// The ontology ID to resolve.
    id: String,
    /// Directory containing ontology definition YAML files.
    ontologies_dir: PathBuf,
}

#[derive(Clone)]
struct Mif;

// rmcp's #[tool] macro requires an instance method (&self receiver) for its
// dispatch mechanism, even though these handlers are stateless.
#[allow(clippy::unused_self)]
#[tool_router]
impl Mif {
    #[tool(description = "Validate a MIF document against the canonical MIF JSON Schema")]
    fn validate_mif_document(
        &self,
        Parameters(ValidateParams { file }): Parameters<ValidateParams>,
    ) -> String {
        let contents = match std::fs::read_to_string(&file) {
            Ok(contents) => contents,
            Err(source) => return format!("failed to read {}: {source}", file.display()),
        };
        let instance: serde_json::Value = match serde_json::from_str(&contents) {
            Ok(instance) => instance,
            Err(source) => return format!("failed to parse {} as JSON: {source}", file.display()),
        };
        match mif_schema::validate_document(&instance) {
            Ok(()) => format!("{}: valid", file.display()),
            Err(error) => {
                let messages = error.messages().join("; ");
                format!("{}: invalid ({messages})", file.display())
            },
        }
    }

    #[tool(description = "Resolve an ontology's three-tier extends chain")]
    fn resolve_ontology_reference(
        &self,
        Parameters(ResolveParams { id, ontologies_dir }): Parameters<ResolveParams>,
    ) -> String {
        let corpus = match mif_ontology::load_corpus_from_dir(&ontologies_dir) {
            Ok(corpus) => corpus,
            Err(error) => return error.to_string(),
        };
        match mif_ontology::resolve_chain(&id, &corpus) {
            Ok(chain) => chain
                .iter()
                .map(|ontology| format!("{} ({})", ontology.id, ontology.version))
                .collect::<Vec<_>>()
                .join(" -> "),
            Err(error) => error.to_string(),
        }
    }
}

#[tool_handler(
    name = "mif-mcp",
    version = "0.1.0",
    instructions = "Validate MIF documents and resolve MIF ontology references"
)]
impl ServerHandler for Mif {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = Mif.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{Mif, Parameters, ResolveParams, ValidateParams};

    fn write_temp_file(contents: &str) -> tempfile::NamedTempFile {
        let file = tempfile::NamedTempFile::new().unwrap();
        fs::write(file.path(), contents).unwrap();
        file
    }

    #[test]
    fn validate_tool_accepts_a_conformant_document() {
        let file = write_temp_file(
            r#"{
                "@context": "https://mif-spec.dev/schema/context.jsonld",
                "@type": "Concept",
                "@id": "urn:mif:memory:test-001",
                "conceptType": "semantic",
                "content": "Test content.",
                "created": "2026-07-02T00:00:00Z"
            }"#,
        );
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: file.path().to_path_buf(),
        }));
        assert!(result.ends_with(": valid"));
    }

    #[test]
    fn validate_tool_reports_invalid_document() {
        let file = write_temp_file(r#"{"content": "missing required fields"}"#);
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: file.path().to_path_buf(),
        }));
        assert!(result.contains("invalid"));
    }

    #[test]
    fn validate_tool_reports_missing_file() {
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: "/nonexistent/mif-mcp-test-fixture.json".into(),
        }));
        assert!(result.contains("failed to read"));
    }

    #[test]
    fn validate_tool_reports_invalid_json() {
        let file = write_temp_file("not json");
        let result = Mif.validate_mif_document(Parameters(ValidateParams {
            file: file.path().to_path_buf(),
        }));
        assert!(result.contains("failed to parse"));
    }

    #[test]
    fn resolve_tool_returns_the_extends_chain() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("mif-base.yaml"),
            "ontology:\n  id: mif-base\n  version: 1.0.0\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("domain.yaml"),
            "ontology:\n  id: domain\n  version: 1.0.0\n  extends: [mif-base]\n",
        )
        .unwrap();
        let result = Mif.resolve_ontology_reference(Parameters(ResolveParams {
            id: "domain".to_string(),
            ontologies_dir: dir.path().to_path_buf(),
        }));
        assert_eq!(result, "mif-base (1.0.0) -> domain (1.0.0)");
    }

    #[test]
    fn resolve_tool_reports_unknown_ontology() {
        let dir = tempfile::tempdir().unwrap();
        let result = Mif.resolve_ontology_reference(Parameters(ResolveParams {
            id: "missing".to_string(),
            ontologies_dir: dir.path().to_path_buf(),
        }));
        assert!(result.contains("not found"));
    }
}
