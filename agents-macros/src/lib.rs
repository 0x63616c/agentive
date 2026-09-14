use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_macro_input, punctuated::Punctuated, token::Comma, Attribute, Expr, FnArg,
    GenericArgument, ImplItem, Item, ItemFn, ItemImpl, Lit, Meta, PathArguments, ReturnType, Type,
    TypePath,
};

#[proc_macro_attribute]
pub fn tool(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attrs = parse_macro_input!(
        attr with Punctuated::<Meta, Comma>::parse_terminated
    );
    let parsed_item = parse_macro_input!(item as Item);

    let options = match parse_tool_attrs(&attrs) {
        Ok(options) => options,
        Err(error) => return error,
    };

    match parsed_item {
        Item::Fn(function) => macro_for_function(options, function),
        Item::Impl(impl_block) => macro_for_impl(options, impl_block),
        _ => compile_error("tool attribute supports free functions or impl blocks"),
    }
}

fn macro_for_function(options: ToolOptions, function: ItemFn) -> TokenStream {
    if function.sig.asyncness.is_none() {
        return compile_error("tool-annotated functions must be async");
    }

    if !function
        .sig
        .inputs
        .iter()
        .all(|arg| !matches!(arg, FnArg::Receiver(_)))
    {
        return compile_error("free function tools must not include a receiver");
    }

    let signature = match callable_args(&function.sig.inputs) {
        Ok(signature) => signature,
        Err(error) => return error,
    };

    let (args_ty, has_context) = signature;

    let function_name = function.sig.ident.clone();
    let wrapper_name = format_ident!("{}Tool", to_pascal_case(&function_name.to_string()));
    let output_ty = match extract_output_type(&function.sig.output) {
        Ok(ty) => ty,
        Err(_) => {
            return compile_error("tool return type must be Result<Output, ToolError>");
        }
    };

    let name_lit = syn::LitStr::new(&options.name, function_name.span());
    let description_lit = syn::LitStr::new(&options.description, function_name.span());
    let call = if has_context {
        quote! { let output: #output_ty = #function_name(context, args).await?; }
    } else {
        quote! { let output: #output_ty = #function_name(args).await?; }
    };
    let idempotent = if options.idempotent {
        quote!(true)
    } else {
        quote!(false)
    };

    let generated = quote! {
        #function

        pub struct #wrapper_name;

        impl agents::Tool for #wrapper_name {
            fn name(&self) -> &agents::ToolName {
                static NAME: std::sync::OnceLock<agents::ToolName> = std::sync::OnceLock::new();
                NAME.get_or_init(|| {
                    agents::ToolName::parse(#name_lit).expect("tool name must be valid")
                })
            }

            fn description(&self) -> &'static str {
                #description_lit
            }

            fn schema_json(&self) -> &serde_json::Value {
                static SCHEMA: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
                SCHEMA.get_or_init(|| {
                    serde_json::to_value(&schemars::schema_for!(#args_ty))
                        .expect("tool schema generation failed")
                })
            }

            fn idempotent(&self) -> bool {
                #idempotent
            }

            fn call<'call>(
                &'call self,
                context: &'call agents::ToolContext,
                args: serde_json::Value,
            ) -> agents::ToolCallFuture<'call> {
                Box::pin(async move {
                    let args: #args_ty = agents::decode_tool_call_args(args)
                        .map_err(|err| {
                            agents::ToolError::terminal(
                                "invalid_arguments",
                                err.to_string(),
                            )
                        })?;

                    #call

                    serde_json::to_value(output).map_err(|err| {
                        agents::ToolError::terminal(
                            "serialize_error",
                            err.to_string(),
                        )
                    })
                })
            }
        }
    };

    generated.into()
}

fn macro_for_impl(options: ToolOptions, impl_block: ItemImpl) -> TokenStream {
    let call_method = impl_block
        .items
        .iter()
        .filter_map(|entry| {
            if let ImplItem::Fn(method) = entry {
                if method.sig.ident == "call" {
                    Some(method.clone())
                } else {
                    None
                }
            } else {
                None
            }
        })
        .next();

    let call_method = match call_method {
        Some(method) => method,
        None => return compile_error("impl tool blocks must contain a `call` method"),
    };

    if call_method.sig.asyncness.is_none() {
        return compile_error("impl tool `call` methods must be async");
    }

    let signature = match callable_args(&call_method.sig.inputs) {
        Ok(signature) => signature,
        Err(error) => return error,
    };
    let (args_ty, has_context) = signature;

    let output_ty = match extract_output_type(&call_method.sig.output) {
        Ok(ty) => ty,
        Err(_) => {
            return compile_error("tool return type must be Result<Output, ToolError>");
        }
    };

    let type_name = impl_block.self_ty.as_ref().clone();
    let call_expr = if has_context {
        quote! { let output: #output_ty = #type_name::call(self, context, args).await?; }
    } else {
        quote! { let output: #output_ty = #type_name::call(self, args).await?; }
    };

    let name_lit = syn::LitStr::new(&options.name, impl_block.brace_token.span.open());
    let description_lit =
        syn::LitStr::new(&options.description, impl_block.brace_token.span.open());
    let idempotent = if options.idempotent {
        quote!(true)
    } else {
        quote!(false)
    };

    let generated = quote! {
        #impl_block

        impl agents::Tool for #type_name {
            fn name(&self) -> &agents::ToolName {
                static NAME: std::sync::OnceLock<agents::ToolName> = std::sync::OnceLock::new();
                NAME.get_or_init(|| {
                    agents::ToolName::parse(#name_lit).expect("tool name must be valid")
                })
            }

            fn description(&self) -> &'static str {
                #description_lit
            }

            fn schema_json(&self) -> &serde_json::Value {
                static SCHEMA: std::sync::OnceLock<serde_json::Value> = std::sync::OnceLock::new();
                SCHEMA.get_or_init(|| {
                    serde_json::to_value(&schemars::schema_for!(#args_ty))
                        .expect("tool schema generation failed")
                })
            }

            fn idempotent(&self) -> bool {
                #idempotent
            }

            fn call<'call>(
                &'call self,
                context: &'call agents::ToolContext,
                args: serde_json::Value,
            ) -> agents::ToolCallFuture<'call> {
                Box::pin(async move {
                    let args: #args_ty = agents::decode_tool_call_args(args)
                        .map_err(|err| {
                            agents::ToolError::terminal(
                                "invalid_arguments",
                                err.to_string(),
                            )
                        })?;

                    #call_expr

                    serde_json::to_value(output).map_err(|err| {
                        agents::ToolError::terminal(
                            "serialize_error",
                            err.to_string(),
                        )
                    })
                })
            }
        }
    };

    generated.into()
}

#[derive(Clone, Debug)]
struct ToolOptions {
    name: String,
    description: String,
    idempotent: bool,
}

fn parse_tool_attrs(attrs: &Punctuated<Meta, Comma>) -> Result<ToolOptions, TokenStream> {
    let mut name: Option<String> = None;
    let mut description: Option<String> = None;
    let mut idempotent = false;

    for meta in attrs {
        match meta {
            Meta::List(list) if list.path.is_ident("tool") => {
                if list.tokens.to_string().trim() == "context" {
                    idempotent = true;
                } else {
                    return Err(compile_error(
                        "tool context marker must be used only in arguments",
                    ));
                }
            }
            Meta::Path(path) => {
                if path.is_ident("idempotent") {
                    idempotent = true;
                }
            }
            Meta::NameValue(name_value) => {
                if name_value.path.is_ident("name") {
                    if let Expr::Lit(expr_lit) = &name_value.value {
                        if let Lit::Str(value) = &expr_lit.lit {
                            name = Some(value.value());
                        }
                    }
                } else if name_value.path.is_ident("description") {
                    if let Expr::Lit(expr_lit) = &name_value.value {
                        if let Lit::Str(value) = &expr_lit.lit {
                            description = Some(value.value());
                        }
                    }
                }
            }
            Meta::List(_list) => {
                return Err(compile_error("unknown tool attribute argument"));
            }
        }
    }

    let name = match name {
        Some(name) => name,
        None => return Err(compile_error("tool `name` is required")),
    };
    let description = match description {
        Some(value) => value,
        None => return Err(compile_error("tool `description` is required")),
    };

    if !validate_tool_name(&name) {
        return Err(compile_error(
            "tool name must be 1..64 alphanumeric, underscore, or hyphen",
        ));
    }

    Ok(ToolOptions {
        name,
        description,
        idempotent,
    })
}

fn callable_args(inputs: &Punctuated<FnArg, Comma>) -> Result<(Type, bool), TokenStream> {
    let mut args: Vec<Type> = Vec::new();
    let mut has_context = false;

    for arg in inputs {
        match arg {
            FnArg::Receiver(_) => continue,
            FnArg::Typed(pat) => {
                let is_context = is_tool_context_arg(pat);
                if is_context {
                    if has_context {
                        return Err(compile_error("tool context may only be declared once"));
                    }
                    has_context = true;
                    continue;
                }

                args.push(*pat.ty.clone());
            }
        }
    }

    if args.len() != 1 {
        return Err(compile_error(
            "tool functions must accept exactly one argument payload (context is optional)",
        ));
    }

    Ok((args[0].clone(), has_context))
}

fn is_tool_context_arg(arg: &syn::PatType) -> bool {
    let has_marker = arg.attrs.iter().any(is_tool_context_attribute);

    has_marker && is_tool_context_type(&arg.ty)
}

fn is_tool_context_attribute(attribute: &Attribute) -> bool {
    if !attribute.path().is_ident("tool") {
        return false;
    }

    match &attribute.meta {
        Meta::List(list) => list.tokens.to_string().trim() == "context",
        Meta::Path(path) => path.is_ident("tool"),
        _ => false,
    }
}

fn is_tool_context_type(ty: &Type) -> bool {
    match ty {
        Type::Reference(reference) => is_tool_context_type(&reference.elem),
        Type::Path(TypePath { path, .. }) => path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "ToolContext"),
        _ => false,
    }
}

fn extract_output_type(signature: &ReturnType) -> Result<Type, TokenStream> {
    let ty = match signature {
        ReturnType::Type(_, ty) => ty.as_ref(),
        ReturnType::Default => {
            return Err(compile_error(
                "tool return type must be Result<..., ToolError>",
            ))
        }
    };

    let Type::Path(path) = ty else {
        return Err(compile_error(
            "tool return type must be Result<..., ToolError>",
        ));
    };

    let result_segment = match path.path.segments.last() {
        Some(segment) => segment,
        None => {
            return Err(compile_error(
                "tool return type must be Result<..., ToolError>",
            ))
        }
    };
    if result_segment.ident != "Result" {
        return Err(compile_error(
            "tool return type must be Result<..., ToolError>",
        ));
    }

    let arguments = match &result_segment.arguments {
        PathArguments::AngleBracketed(args) => &args.args,
        _ => {
            return Err(compile_error(
                "tool return type must be Result<Output, ToolError>",
            ))
        }
    };
    let mut type_args = arguments.iter();
    let first = match type_args.next() {
        Some(argument) => argument,
        None => {
            return Err(compile_error(
                "tool return type must be Result<Output, ToolError>",
            ));
        }
    };
    let second = match type_args.next() {
        Some(argument) => argument,
        None => {
            return Err(compile_error(
                "tool return type must be Result<Output, ToolError>",
            ));
        }
    };

    let GenericArgument::Type(Type::Path(output)) = first else {
        return Err(compile_error(
            "tool return type output type must be concrete",
        ));
    };

    let error_segment = match second {
        GenericArgument::Type(Type::Path(path)) => match path.path.segments.last() {
            Some(segment) => segment,
            None => {
                return Err(compile_error(
                    "tool return type must be Result<Output, ToolError>",
                ))
            }
        },
        _ => {
            return Err(compile_error(
                "tool return type must be Result<Output, ToolError>",
            ))
        }
    };
    if error_segment.ident != "ToolError" {
        return Err(compile_error("tool error type must be ToolError"));
    }

    Ok(Type::Path(output.clone()))
}

fn validate_tool_name(value: &str) -> bool {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-');
    valid
}

fn compile_error(message: &str) -> TokenStream {
    let token = syn::Error::new(proc_macro2::Span::call_site(), message).to_compile_error();
    TokenStream::from(token)
}

fn to_pascal_case(value: &str) -> String {
    let mut output = String::new();
    let mut capitalize = true;

    for ch in value.chars() {
        if ch == '_' || ch == '-' {
            capitalize = true;
            continue;
        }
        if capitalize {
            output.push(ch.to_ascii_uppercase());
            capitalize = false;
        } else {
            output.push(ch);
        }
    }

    output
}
