//! JSON Schema validation for tool arguments.

use serde_json::Value;

#[derive(Debug)]
/// Result of compiling a JSON Schema and validating an argument value.
pub enum SchemaValidationOutcome {
    /// The arguments satisfy the schema.
    Valid,
    /// The schema compiled, but the arguments violate one or more constraints.
    Invalid {
        /// All validation errors reported by the schema validator.
        errors: Vec<String>,
    },
    /// The supplied schema could not be compiled.
    BadSchema {
        /// Description of the schema compilation failure.
        error: String,
    },
}

/// Validate `arguments` against `schema` and collect every violation.
///
/// Schema compilation failures are distinguished from argument validation
/// failures through [`SchemaValidationOutcome::BadSchema`].
pub fn validate_arguments(schema: &Value, arguments: &Value) -> SchemaValidationOutcome {
    let validator = match jsonschema::validator_for(schema) {
        Ok(v) => v,
        Err(e) => {
            return SchemaValidationOutcome::BadSchema {
                error: e.to_string(),
            };
        }
    };

    // Collect all validation errors
    let errors: Vec<String> = validator
        .iter_errors(arguments)
        .map(|e| e.to_string())
        .collect();

    if errors.is_empty() {
        SchemaValidationOutcome::Valid
    } else {
        SchemaValidationOutcome::Invalid { errors }
    }
}
