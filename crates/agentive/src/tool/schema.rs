use super::ToolDecodeError;
use schemars::{JsonSchema, schema_for};
use serde_json::Value;

/// Decode validated tool arguments, rejecting fields outside the schema.
pub fn decode_tool_call_args<T: serde::de::DeserializeOwned + JsonSchema>(
    value: Value,
) -> Result<T, ToolDecodeError> {
    validate_tool_call_schema::<T>(&value)?;
    serde_json::from_value::<T>(value)
        .map_err(|error| ToolDecodeError::UnknownFields(error.to_string()))
}
fn validate_tool_call_schema<T: JsonSchema>(value: &Value) -> Result<(), ToolDecodeError> {
    let schema = serde_json::to_value(schema_for!(T))
        .map_err(|error| ToolDecodeError::InvalidSchema(error.to_string()))?;
    validate_schema_shape(&schema)?;
    validate_unknown_fields(value, &schema)
}
fn validate_schema_shape(schema: &Value) -> Result<(), ToolDecodeError> {
    let schema = schema.get("schema").unwrap_or(schema);
    if schema.get("type").and_then(Value::as_str) == Some("object") {
        if !matches!(schema.get("additionalProperties"), Some(Value::Bool(false))) {
            return Err(ToolDecodeError::InvalidSchema(
                "tool argument schema must reject unknown fields".to_string(),
            ));
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (name, child) in properties {
                let described = child
                    .get("description")
                    .and_then(Value::as_str)
                    .is_some_and(|value| !value.trim().is_empty())
                    || child
                        .get("metadata")
                        .and_then(|metadata| metadata.get("description"))
                        .and_then(Value::as_str)
                        .is_some_and(|value| !value.trim().is_empty());
                if !described {
                    return Err(ToolDecodeError::InvalidSchema(format!(
                        "property `{name}` missing description"
                    )));
                }
                validate_schema_shape(child)?;
            }
        } else {
            return Ok(());
        }
    }
    if let Some(items) = schema.get("items") {
        validate_schema_shape(items)?;
    }
    if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
        for child in all_of {
            validate_schema_shape(child)?;
        }
    }
    Ok(())
}
fn validate_unknown_fields(value: &Value, schema: &Value) -> Result<(), ToolDecodeError> {
    validate_unknown_fields_inner(value, schema, schema)
}
fn resolve_ref<'a>(schema: &'a Value, root: &'a Value) -> &'a Value {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str)
        && let Some(leaf) = reference
            .strip_prefix("#/$defs/")
            .or_else(|| reference.strip_prefix("#/$ref/"))
        && let Some(child) = root
            .get("$defs")
            .and_then(Value::as_object)
            .and_then(|defs| defs.get(leaf))
    {
        return child;
    }
    schema
}
fn validate_unknown_fields_inner(
    value: &Value,
    schema: &Value,
    root: &Value,
) -> Result<(), ToolDecodeError> {
    let schema = resolve_ref(schema, root);
    let schema = schema.get("schema").unwrap_or(schema);
    match (value, schema.get("type").and_then(Value::as_str)) {
        (Value::Object(values), Some("object")) => {
            if !matches!(schema.get("additionalProperties"), Some(Value::Bool(false))) {
                return Err(ToolDecodeError::UnknownFields(
                    "unknown object fields must be rejected".to_string(),
                ));
            }
            let properties = schema
                .get("properties")
                .and_then(Value::as_object)
                .ok_or_else(|| {
                    ToolDecodeError::InvalidSchema(
                        "tool argument schema for object values must include properties"
                            .to_string(),
                    )
                })?;
            for (name, child_value) in values {
                let child = properties.get(name).ok_or_else(|| {
                    ToolDecodeError::UnknownFields(format!("unknown field `{name}` in tool args"))
                })?;
                validate_unknown_fields_inner(child_value, child, root)?;
            }
        }
        (Value::Array(items), Some("array")) => {
            if let Some(items_schema) = schema.get("items") {
                for item in items {
                    validate_unknown_fields_inner(item, items_schema, root)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}
