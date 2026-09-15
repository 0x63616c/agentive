#![allow(missing_docs)]

#[test]
fn tool_macro_compile_contracts() {
    let t = trybuild::TestCases::new();
    t.pass("tests/tool_macro/ok.rs");
    t.pass("tests/tool_macro/stateful.rs");
    t.pass("tests/tool_macro/custom_error.rs");
    t.compile_fail("tests/tool_macro/missing_description.rs");
    t.compile_fail("tests/tool_macro/invalid_name.rs");
    t.compile_fail("tests/tool_macro/invalid_signature.rs");
    t.compile_fail("tests/tool_macro/invalid_return.rs");
    t.compile_fail("tests/tool_macro/context_after_arguments.rs");
    t.compile_fail("tests/tool_macro/malformed_context_marker.rs");
}
