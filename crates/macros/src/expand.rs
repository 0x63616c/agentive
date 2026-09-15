//! Token generation for stateless and stateful tools.

use crate::{
    attributes::ToolOptions,
    diagnostics::{Error, error},
    signature::{ToolSignature, parse, remove_context_markers},
};
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ImplItem, ImplItemFn, ItemFn, ItemImpl};

pub(crate) fn free_function(
    options: ToolOptions,
    mut function: ItemFn,
) -> Result<TokenStream, Error> {
    if function.sig.asyncness.is_none() {
        return Err(error(
            &function.sig,
            "tool-annotated functions must be async",
        ));
    }
    if function
        .sig
        .inputs
        .iter()
        .any(|input| matches!(input, FnArg::Receiver(_)))
    {
        return Err(error(
            &function.sig,
            "free function tools must not include a receiver",
        ));
    }
    let signature = parse(&function.sig.inputs, &function.sig.output)?;
    remove_context_markers(&mut function.sig.inputs);
    let function_name = function.sig.ident.clone();
    let wrapper_name = format_ident!("{}Tool", pascal_case(&function_name.to_string()));
    let invocation = if signature.has_context {
        quote!(#function_name(context, args).await)
    } else {
        quote!(#function_name(args).await)
    };
    let implementation = tool_implementation(&wrapper_name, &options, &signature, invocation);
    Ok(quote! { #function pub struct #wrapper_name; #implementation })
}

pub(crate) fn implementation(
    options: ToolOptions,
    mut implementation: ItemImpl,
) -> Result<TokenStream, Error> {
    let call_method = find_call_method(&implementation)?;
    if call_method.sig.asyncness.is_none() {
        return Err(error(
            &call_method.sig,
            "impl tool `call` methods must be async",
        ));
    }
    if !matches!(call_method.sig.inputs.first(), Some(FnArg::Receiver(receiver)) if receiver.reference.is_some() && receiver.mutability.is_none())
    {
        return Err(error(
            &call_method.sig,
            "impl tool `call` methods must take `&self` as their receiver",
        ));
    }
    let signature = parse(&call_method.sig.inputs, &call_method.sig.output)?;
    for item in &mut implementation.items {
        if let ImplItem::Fn(method) = item
            && method.sig.ident == "call"
        {
            remove_context_markers(&mut method.sig.inputs);
        }
    }
    let type_name = implementation.self_ty.as_ref();
    let invocation = if signature.has_context {
        quote!(Self::call(self, context, args).await)
    } else {
        quote!(Self::call(self, args).await)
    };
    let tool_implementation = tool_implementation(type_name, &options, &signature, invocation);
    Ok(quote! { #implementation #tool_implementation })
}

fn find_call_method(implementation: &ItemImpl) -> Result<ImplItemFn, Error> {
    let mut methods = implementation.items.iter().filter_map(|item| match item {
        ImplItem::Fn(method) if method.sig.ident == "call" => Some(method),
        _ => None,
    });
    let Some(call_method) = methods.next() else {
        return Err(error(
            implementation,
            "impl tool blocks must contain an async `call` method",
        ));
    };
    if let Some(duplicate) = methods.next() {
        return Err(error(
            duplicate,
            "impl tool blocks may contain only one `call` method",
        ));
    }
    Ok(call_method.clone())
}

fn tool_implementation(
    type_name: &impl quote::ToTokens,
    options: &ToolOptions,
    signature: &ToolSignature,
    invocation: TokenStream,
) -> TokenStream {
    let name = syn::LitStr::new(&options.name, proc_macro2::Span::call_site());
    let description = syn::LitStr::new(&options.description, proc_macro2::Span::call_site());
    let idempotent = options.idempotent;
    let args_ty = &signature.args_ty;
    let output_ty = &signature.output_ty;
    quote! {
        impl agentive::Tool for #type_name {
            fn name(&self) -> &agentive::ToolName {
                static NAME: std::sync::OnceLock<agentive::ToolName> = std::sync::OnceLock::new();
                NAME.get_or_init(|| agentive::ToolName::parse(#name).expect("validated tool name"))
            }
            fn description(&self) -> &'static str { #description }
            fn schema_json(&self) -> &serde_json::Value {
                static SCHEMA: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
                SCHEMA.get_or_init(|| {
                    serde_json::to_value(&schemars::schema_for!(#args_ty))
                        .expect("JSON Schema serialization is infallible for schemars output")
                })
            }
            fn idempotent(&self) -> bool { #idempotent }
            fn call<'call>(
                &'call self,
                context: &'call agentive::ToolContext,
                args: serde_json::Value,
            ) -> agentive::ToolCallFuture<'call> {
                Box::pin(async move {
                    let args: #args_ty = agentive::decode_tool_call_args(args).map_err(|error| {
                        agentive::ToolError::terminal("invalid_arguments", error.to_string())
                    })?;
                    let output: #output_ty = (#invocation)
                        .map_err(|error| -> agentive::ToolError { error.into() })?;
                    serde_json::to_value(output).map_err(|error| {
                        agentive::ToolError::terminal("serialize_error", error.to_string())
                    })
                })
            }
        }
    }
}

fn pascal_case(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut capitalize = true;
    for character in value.chars() {
        if matches!(character, '_' | '-') {
            capitalize = true;
        } else if capitalize {
            output.push(character.to_ascii_uppercase());
            capitalize = false;
        } else {
            output.push(character);
        }
    }
    output
}
