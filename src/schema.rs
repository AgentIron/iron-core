use serde_json::Value;

#[derive(Debug)]
pub enum SchemaValidationOutcome {
    Valid,
    Invalid { errors: Vec<String> },
    BadSchema { error: String },
}

pub fn validate_arguments(schema: &Value, arguments: &Value) -> SchemaValidationOutcome {
    let validator = match jsonschema::validator_for(schema) {
        Ok(v) => v,
        Err(e) => {
            return SchemaValidationOutcome::BadSchema {
                error: e.to_string(),
            }
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
