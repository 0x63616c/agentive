use super::ToolDecodeError;
use schemars::{JsonSchema, schema_for};
use serde_json::Value;

/// Validates the same schema that is published to providers and used for decoding.
pub fn validate_tool_definition(description: &str, schema: &Value) -> Result<(), ToolDecodeError> {
    if description.trim().is_empty() {
        return Err(ToolDecodeError::InvalidSchema(
            "tool description must not be blank".to_string(),
        ));
    }
    let schema = schema.get("schema").unwrap_or(schema);
    jsonschema::validator_for(schema).map_err(|_| {
        ToolDecodeError::InvalidSchema("tool argument schema could not be compiled".to_string())
    })?;
    let top_level = resolve_ref(schema, schema);
    if top_level.get("type").and_then(Value::as_str) != Some("object") {
        return Err(ToolDecodeError::InvalidSchema(
            "tool argument schema must describe a top-level object".to_string(),
        ));
    }
    if !matches!(
        top_level.get("additionalProperties"),
        Some(Value::Bool(false))
    ) {
        return Err(ToolDecodeError::InvalidSchema(
            "tool argument schema must reject unknown fields".to_string(),
        ));
    }
    validate_schema_shape(schema)
}

/// Decode validated tool arguments, rejecting fields outside the schema.
pub fn decode_tool_call_args<T: serde::de::DeserializeOwned + JsonSchema>(
    value: Value,
) -> Result<T, ToolDecodeError> {
    validate_tool_call_schema::<T>(&value)?;
    serde_json::from_value::<T>(value)
        .map_err(|error| ToolDecodeError::UnknownFields(error.to_string()))
}

/// Validates untrusted arguments against an already validated published schema.
pub fn validate_tool_arguments(value: &Value, schema: &Value) -> Result<(), ToolDecodeError> {
    let schema = schema.get("schema").unwrap_or(schema);
    let validator = jsonschema::validator_for(schema).map_err(|_| {
        ToolDecodeError::InvalidSchema("tool argument schema could not be compiled".to_string())
    })?;
    validator.validate(value).map_err(|_| {
        ToolDecodeError::UnknownFields("arguments do not match the tool schema".to_string())
    })
}
fn validate_tool_call_schema<T: JsonSchema>(value: &Value) -> Result<(), ToolDecodeError> {
    let schema = serde_json::to_value(schema_for!(T))
        .map_err(|error| ToolDecodeError::InvalidSchema(error.to_string()))?;
    validate_tool_definition("typed tool arguments", &schema)?;
    validate_unknown_fields(value, &schema)
}
fn validate_schema_shape(schema: &Value) -> Result<(), ToolDecodeError> {
    validate_schema_shape_inner(
        schema.get("schema").unwrap_or(schema),
        schema,
        &mut Vec::new(),
    )
}
fn validate_schema_shape_inner(
    schema: &Value,
    root: &Value,
    refs: &mut Vec<String>,
) -> Result<(), ToolDecodeError> {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        if refs.iter().any(|seen| seen == reference) {
            return Ok(());
        }
        let Some(name) = reference.strip_prefix("#/$defs/") else {
            return Err(ToolDecodeError::InvalidSchema(
                "only local $defs references are supported".into(),
            ));
        };
        let resolved = root
            .get("$defs")
            .and_then(Value::as_object)
            .and_then(|defs| defs.get(name))
            .ok_or_else(|| {
                ToolDecodeError::InvalidSchema(format!("unresolved schema reference `{reference}`"))
            })?;
        refs.push(reference.to_string());
        let result = validate_schema_shape_inner(resolved, root, refs);
        refs.pop();
        return result;
    }
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
                validate_schema_shape_inner(child, root, refs)?;
            }
        } else {
            return Ok(());
        }
    }
    if let Some(items) = schema.get("items") {
        validate_schema_shape_inner(items, root, refs)?;
    }
    if let Some(all_of) = schema.get("allOf").and_then(Value::as_array) {
        for child in all_of {
            validate_schema_shape_inner(child, root, refs)?;
        }
    }
    for key in ["anyOf", "oneOf", "not", "if", "then", "else"] {
        match schema.get(key) {
            Some(Value::Array(children)) => {
                for child in children {
                    validate_schema_shape_inner(child, root, refs)?;
                }
            }
            Some(child) => validate_schema_shape_inner(child, root, refs)?,
            None => {}
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
