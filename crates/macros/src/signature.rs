//! Validation and extraction of supported tool call signatures.

use crate::diagnostics::{Error, error};
use syn::{
    Attribute, FnArg, GenericArgument, PatType, PathArguments, ReturnType, Type, TypePath,
    punctuated::Punctuated, token::Comma,
};

pub(crate) struct ToolSignature {
    pub(crate) args_ty: Type,
    pub(crate) output_ty: Type,
    pub(crate) has_context: bool,
}

pub(crate) fn parse(
    inputs: &Punctuated<FnArg, Comma>,
    output: &ReturnType,
) -> Result<ToolSignature, Error> {
    let (args_ty, has_context) = parse_inputs(inputs)?;
    let output_ty = parse_output(output)?;
    Ok(ToolSignature {
        args_ty,
        output_ty,
        has_context,
    })
}

pub(crate) fn remove_context_markers(inputs: &mut Punctuated<FnArg, Comma>) {
    for input in inputs {
        if let FnArg::Typed(argument) = input {
            argument
                .attrs
                .retain(|attribute| !is_context_marker(attribute));
        }
    }
}

fn parse_inputs(inputs: &Punctuated<FnArg, Comma>) -> Result<(Type, bool), Error> {
    let mut args_ty = None;
    let mut has_context = false;
    for input in inputs {
        let FnArg::Typed(argument) = input else {
            continue;
        };
        if has_tool_attribute(argument) && !is_context_marker_present(argument) {
            return Err(error(
                argument,
                "tool context parameters must use exactly `#[tool(context)]`",
            ));
        }
        if is_context_marker_present(argument) {
            if !is_tool_context(&argument.ty) {
                return Err(error(
                    argument,
                    "a `#[tool(context)]` parameter must be `&ToolContext`",
                ));
            }
            if has_context {
                return Err(error(argument, "tool context may only be declared once"));
            }
            if args_ty.is_some() {
                return Err(error(
                    argument,
                    "tool context must appear before the argument payload",
                ));
            }
            has_context = true;
        } else if args_ty.is_some() {
            return Err(error(
                argument,
                "tool functions must accept exactly one argument payload (context is optional)",
            ));
        } else {
            args_ty = Some((*argument.ty).clone());
        }
    }
    let args_ty = args_ty.ok_or_else(|| {
        Error::new(
            proc_macro2::Span::call_site(),
            "tool functions must accept exactly one argument payload (context is optional)",
        )
    })?;
    Ok((args_ty, has_context))
}

fn parse_output(output: &ReturnType) -> Result<Type, Error> {
    let ReturnType::Type(_, ty) = output else {
        return Err(error(
            output,
            "tool return type must be Result<Output, ToolError>",
        ));
    };
    let Type::Path(path) = ty.as_ref() else {
        return Err(error(
            ty,
            "tool return type must be Result<Output, ToolError>",
        ));
    };
    let Some(segment) = path.path.segments.last() else {
        return Err(error(
            path,
            "tool return type must be Result<Output, ToolError>",
        ));
    };
    if segment.ident != "Result" {
        return Err(error(
            segment,
            "tool return type must be Result<Output, ToolError>",
        ));
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return Err(error(
            segment,
            "tool return type must be Result<Output, ToolError>",
        ));
    };
    if arguments.args.len() != 2 {
        return Err(error(
            arguments,
            "tool return type must be Result<Output, ToolError>",
        ));
    }
    let mut types = arguments.args.iter();
    let Some(GenericArgument::Type(output)) = types.next() else {
        return Err(error(
            arguments,
            "tool return type output type must be concrete",
        ));
    };
    let Some(GenericArgument::Type(_error)) = types.next() else {
        return Err(error(
            arguments,
            "tool return type must be Result<Output, ToolError>",
        ));
    };
    Ok(output.clone())
}

fn is_context_marker_present(argument: &PatType) -> bool {
    argument.attrs.iter().any(is_context_marker)
}

fn has_tool_attribute(argument: &PatType) -> bool {
    argument
        .attrs
        .iter()
        .any(|attribute| attribute.path().is_ident("tool"))
}

fn is_context_marker(attribute: &Attribute) -> bool {
    attribute.path().is_ident("tool")
        && attribute
            .meta
            .require_list()
            .is_ok_and(|list| list.tokens.to_string().trim() == "context")
}

fn is_tool_context(ty: &Type) -> bool {
    matches!(ty, Type::Reference(reference) if matches!(reference.elem.as_ref(), Type::Path(TypePath { path, .. }) if path.segments.last().is_some_and(|segment| segment.ident == "ToolContext")))
}
