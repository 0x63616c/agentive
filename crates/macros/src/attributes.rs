//! Parsing for explicit `#[tool(...)]` options.

use crate::diagnostics::{Error, error};
use syn::{Expr, Lit, Meta, punctuated::Punctuated, token::Comma};

#[derive(Clone, Debug)]
pub(crate) struct ToolOptions {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) idempotent: bool,
}

impl ToolOptions {
    pub(crate) fn parse(attributes: &Punctuated<Meta, Comma>) -> Result<Self, Error> {
        let mut name = None;
        let mut description = None;
        let mut idempotent = false;

        for attribute in attributes {
            match attribute {
                Meta::Path(path) if path.is_ident("idempotent") => {
                    if idempotent {
                        return Err(error(path, "tool `idempotent` may only be declared once"));
                    }
                    idempotent = true;
                }
                Meta::NameValue(value) if value.path.is_ident("name") => {
                    if name.is_some() {
                        return Err(error(value, "tool `name` may only be declared once"));
                    }
                    name = Some(parse_literal(
                        &value.value,
                        "tool `name` must be a string literal",
                    )?);
                }
                Meta::NameValue(value) if value.path.is_ident("description") => {
                    if description.is_some() {
                        return Err(error(value, "tool `description` may only be declared once"));
                    }
                    description = Some(parse_literal(
                        &value.value,
                        "tool `description` must be a string literal",
                    )?);
                }
                Meta::List(list) if list.path.is_ident("tool") => {
                    return Err(error(
                        list,
                        "tool context marker must be used only on context arguments",
                    ));
                }
                _ => return Err(error(attribute, "unknown tool attribute argument")),
            }
        }

        let name = name
            .ok_or_else(|| Error::new(proc_macro2::Span::call_site(), "tool `name` is required"))?;
        let description = description.ok_or_else(|| {
            Error::new(
                proc_macro2::Span::call_site(),
                "tool `description` is required",
            )
        })?;

        if !is_valid_name(&name) {
            return Err(Error::new(
                proc_macro2::Span::call_site(),
                "tool name must be 1..64 ASCII alphanumeric characters, underscores, or hyphens",
            ));
        }
        if description.trim().is_empty() {
            return Err(Error::new(
                proc_macro2::Span::call_site(),
                "tool `description` must not be empty",
            ));
        }

        Ok(Self {
            name,
            description,
            idempotent,
        })
    }
}

fn parse_literal(value: &Expr, message: &str) -> Result<String, Error> {
    let Expr::Lit(literal) = value else {
        return Err(error(value, message));
    };
    let Lit::Str(value) = &literal.lit else {
        return Err(error(literal, message));
    };
    Ok(value.value())
}

fn is_valid_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}
