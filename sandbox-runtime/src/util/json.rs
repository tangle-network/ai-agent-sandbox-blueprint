use serde_json::{Map, Value};

use crate::error::{Result, SandboxError};

pub fn parse_json_object(value: &str, field_name: &str) -> Result<Option<Value>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let parsed: Value = serde_json::from_str(trimmed).map_err(|err| {
        SandboxError::Validation(format!("{field_name} is not valid JSON: {err}"))
    })?;

    if !parsed.is_object() {
        return Err(SandboxError::Validation(format!(
            "{field_name} must be a JSON object"
        )));
    }

    Ok(Some(parsed))
}

pub fn merge_metadata(
    mut metadata: Option<Value>,
    image: &str,
    stack: &str,
) -> Result<Option<Value>> {
    if image.is_empty() && stack.is_empty() {
        return Ok(metadata);
    }

    let mut object = match metadata.take() {
        Some(Value::Object(map)) => map,
        Some(_) => {
            return Err(SandboxError::Validation(
                "metadata_json must be a JSON object".into(),
            ));
        }
        None => Map::new(),
    };

    if !image.is_empty() {
        object.insert("image".to_string(), Value::String(image.to_string()));
    }

    if !stack.is_empty() {
        object.insert("stack".to_string(), Value::String(stack.to_string()));
    }

    Ok(Some(Value::Object(object)))
}
