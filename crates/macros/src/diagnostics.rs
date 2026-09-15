//! Spanned diagnostics for the `#[tool]` parser.

pub(crate) type Error = syn::Error;

pub(crate) fn error<T: quote::ToTokens>(tokens: T, message: &str) -> Error {
    Error::new_spanned(tokens, message)
}
