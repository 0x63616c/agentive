#[test]
fn tool_macro_compile_contracts() {
    let t = trybuild::TestCases::new();
    t.pass("tests/tool_macro/ok.rs");
    t.compile_fail("tests/tool_macro/missing_description.rs");
}
