#![allow(missing_docs)] // Integration-test crate; the contract API is documented in the library.

use agentive_test::{ScriptedProviderHarness, assert_provider_conformance};

#[tokio::test]
async fn scripted_provider_passes_the_reusable_provider_contract()
-> Result<(), Box<dyn std::error::Error>> {
    assert_provider_conformance(&ScriptedProviderHarness).await?;
    Ok(())
}
