//! Generation and parity checks for the published claims.json schema.

use std::io::Write;

use schemars::generate::SchemaSettings;
use serde_json::Value;

use crate::catalog::{Catalog, SCHEMA_VERSION};

pub const ID: &str = "https://hypotheosis.github.io/claims-mcp/schema/claims.schema.json";

/// Generates the canonical JSON Schema 2020-12 document for a catalog.
pub fn catalog_schema() -> Value {
    let schema = SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<Catalog>();
    let mut value = serde_json::to_value(schema).expect("a schema always serializes");
    let object = value.as_object_mut().expect("the root schema is an object");
    object.insert("$id".into(), ID.into());
    object.insert("title".into(), "claims-mcp catalog".into());
    object.insert("x-schema-version".into(), SCHEMA_VERSION.into());
    value
}

/// Writes the canonical, pretty-printed schema with a trailing newline.
pub fn write(mut out: impl Write) -> anyhow::Result<()> {
    serde_json::to_writer_pretty(&mut out, &catalog_schema())?;
    writeln!(out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMITTED: &str = include_str!("../docs/site/src/schema/claims.schema.json");
    const VERSIONED: &str = include_str!("../docs/site/src/schema/v2/claims.schema.json");
    const CATALOG: &str = include_str!("../claims.json");

    #[test]
    fn committed_schemas_match_generated_output() {
        let mut generated = Vec::new();
        write(&mut generated).expect("generate schema");
        let generated = String::from_utf8(generated).expect("schema is UTF-8");
        assert_eq!(COMMITTED, generated, "run `just schema`");
        assert_eq!(VERSIONED, generated, "run `just schema`");
    }

    #[test]
    fn schema_is_valid_and_accepts_repository_catalog() {
        let schema = catalog_schema();
        jsonschema::meta::validate(&schema).expect("generated schema is valid draft 2020-12");
        let validator = jsonschema::validator_for(&schema).expect("compile catalog schema");
        let catalog: Value = serde_json::from_str(CATALOG).expect("parse claims.json");
        if let Err(error) = validator.validate(&catalog) {
            panic!("claims.json does not match its schema: {error}");
        }
    }
}
