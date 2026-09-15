//! Mechanical implementation of the public `agentive::tool` attribute.
//!
//! This crate deliberately contains only syntax validation and the projection from an ordinary
//! Rust function or implementation to `agentive::Tool`. Runtime policy stays in `agentive`.
#![allow(missing_docs)] // The public macro is documented by the facade crate.

mod attributes;
mod diagnostics;
mod expand;
mod signature;

use proc_macro::TokenStream;
use syn::{Item, parse_macro_input, punctuated::Punctuated, token::Comma};

/// Projects an explicit async tool function or implementation into `agentive::Tool`.
#[proc_macro_attribute]
pub fn tool(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attributes = parse_macro_input!(attr with Punctuated::<syn::Meta, Comma>::parse_terminated);
    let item = parse_macro_input!(item as Item);

    let options = match attributes::ToolOptions::parse(&attributes) {
        Ok(options) => options,
        Err(error) => return error.into_compile_error().into(),
    };

    let expanded = match item {
        Item::Fn(function) => expand::free_function(options, function),
        Item::Impl(implementation) => expand::implementation(options, implementation),
        other => Err(diagnostics::Error::new_spanned(
            other,
            "tool attribute supports free functions or impl blocks",
        )),
    };

    expanded
        .unwrap_or_else(diagnostics::Error::into_compile_error)
        .into()
}
