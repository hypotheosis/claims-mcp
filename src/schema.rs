//! Generation and parity checks for the published claims.json schema.

use std::io::Write;

use schemars::generate::SchemaSettings;
use serde_json::Value;

use crate::catalog::{Catalog, SCHEMA_VERSION};

pub const LATEST_ID: &str = "https://hypotheosis.github.io/claims-mcp/schema/claims.schema.json";

fn versioned_id() -> String {
    format!("https://hypotheosis.github.io/claims-mcp/schema/v{SCHEMA_VERSION}/claims.schema.json")
}

/// Generates the canonical JSON Schema 2020-12 document for a catalog.
pub fn catalog_schema(id: &str) -> Value {
    let schema = SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<Catalog>();
    let mut value = serde_json::to_value(schema).expect("a schema always serializes");
    let object = value.as_object_mut().expect("the root schema is an object");
    object.insert("$id".into(), id.into());
    object.insert("title".into(), "claims-mcp catalog".into());
    object.insert("x-schema-version".into(), SCHEMA_VERSION.into());
    value
}

/// Writes the canonical, pretty-printed schema with a trailing newline.
pub fn write(mut out: impl Write, versioned: bool) -> anyhow::Result<()> {
    let id = if versioned {
        versioned_id()
    } else {
        LATEST_ID.to_owned()
    };
    serde_json::to_writer_pretty(&mut out, &catalog_schema(&id))?;
    writeln!(out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMITTED: &str = include_str!("../docs/site/src/schema/claims.schema.json");
    const CATALOG: &str = include_str!("../claims.json");

    #[test]
    fn committed_schemas_match_generated_output() {
        let mut generated = Vec::new();
        write(&mut generated, false).expect("generate latest schema");
        let generated = String::from_utf8(generated).expect("schema is UTF-8");
        assert_eq!(COMMITTED, generated, "run `just schema`");

        let path = format!("docs/site/src/schema/v{SCHEMA_VERSION}/claims.schema.json");
        let committed = std::fs::read_to_string(path).expect("read current versioned schema");
        let mut generated = Vec::new();
        write(&mut generated, true).expect("generate versioned schema");
        assert_eq!(committed.as_bytes(), generated, "run `just schema`");
    }

    #[test]
    fn schema_is_valid_and_accepts_repository_catalog() {
        let schema = catalog_schema(LATEST_ID);
        jsonschema::meta::validate(&schema).expect("generated schema is valid draft 2020-12");
        let version = &schema["properties"]["schema_version"];
        assert_eq!(version["minimum"], SCHEMA_VERSION);
        assert_eq!(version["maximum"], SCHEMA_VERSION);
        let validator = jsonschema::validator_for(&schema).expect("compile catalog schema");
        let catalog: Value = serde_json::from_str(CATALOG).expect("parse claims.json");
        if let Err(error) = validator.validate(&catalog) {
            panic!("claims.json does not match its schema: {error}");
        }
    }
}
