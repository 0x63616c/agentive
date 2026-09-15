#![allow(missing_docs, clippy::expect_used)]

use agentive::{
    AllOrError, CompiledInstructions, InstructionFragment, Message, ModelCapabilities,
    ModelRequest, ProviderToolDescriptor, ToolName, UsageEstimator,
};

fn complete_request() -> ModelRequest {
    ModelRequest {
        instructions: CompiledInstructions {
            core: InstructionFragment::new("core", 1, "core instruction"),
            agent: Some("agent instruction".repeat(64)),
            run: Some("run instruction".repeat(64)),
            features: vec![InstructionFragment::new(
                "feature",
                1,
                "feature instruction",
            )],
        },
        messages: vec![
            Message::user("user message".repeat(64)),
            Message::image_inline(vec![42; 512], "image/png").expect("valid image"),
        ],
        tools: vec![ProviderToolDescriptor {
            name: ToolName::parse("search").expect("test tool name is valid"),
            description: "tool description".repeat(128),
            schema: serde_json::json!({
                "type": "object",
                "description": "schema description".repeat(128),
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "field description".repeat(128)
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
            idempotent: true,
        }],
        include_context: true,
        model: Some("test-model".to_string()),
        max_output_tokens: 99,
        output_format: agentive::ModelOutputFormat::Text,
        invocation_id: "test-invocation".to_string(),
    }
}

#[test]
fn image_messages_reject_ambiguous_or_unsafe_inputs_at_construction_and_deserialization() {
    assert!(Message::image_url("file:///etc/passwd", "image/png").is_err());
    assert!(Message::image_url("https://user:secret@example.test/a.png", "image/png").is_err());
    assert!(Message::image_url("https://example.test/a.svg", "image/svg+xml").is_err());
    assert!(Message::image_inline(Vec::new(), "image/png").is_err());

    let malformed = serde_json::json!({
        "role": "user",
        "content": [{
            "type": "image",
            "source": {"kind": "inline", "bytes": []},
            "media_type": "image/png"
        }]
    });
    assert!(serde_json::from_value::<Message>(malformed).is_err());
}

#[test]
fn conservative_bound_covers_every_rendered_request_layer_and_output_reserve() {
    let estimator = UsageEstimator;
    let complete = complete_request();
    let baseline = ModelRequest {
        instructions: CompiledInstructions {
            core: InstructionFragment::new("core", 1, "core instruction"),
            agent: None,
            run: None,
            features: Vec::new(),
        },
        messages: vec![Message::user("user")],
        tools: Vec::new(),
        include_context: false,
        model: None,
        max_output_tokens: complete.max_output_tokens,
        output_format: agentive::ModelOutputFormat::Text,
        invocation_id: "test-invocation".to_string(),
    };

    let complete_bound = estimator
        .conservative_context_token_bound(&complete)
        .expect("request serializes");
    let baseline_bound = estimator
        .conservative_context_token_bound(&baseline)
        .expect("request serializes");

    assert!(complete_bound > baseline_bound);
    let capabilities = ModelCapabilities {
        max_context_tokens: Some(complete_bound.saturating_add(complete.max_output_tokens)),
        ..Default::default()
    };
    assert!(
        estimator
            .can_admit(
                AllOrError::Conservative,
                &complete,
                &capabilities,
                Some(complete_bound),
                false,
            )
            .is_ok()
    );
    let too_small = ModelCapabilities {
        max_context_tokens: Some(
            complete_bound
                .saturating_add(complete.max_output_tokens)
                .saturating_sub(1),
        ),
        ..Default::default()
    };
    assert!(
        estimator
            .can_admit(
                AllOrError::Conservative,
                &complete,
                &too_small,
                Some(complete_bound),
                false,
            )
            .is_err()
    );
}

#[test]
fn exact_admission_requires_a_provider_model_counter_not_capability_metadata() {
    let estimator = UsageEstimator;
    let request = complete_request();
    let capabilities = ModelCapabilities {
        max_context_tokens: Some(u64::MAX),
        ..Default::default()
    };

    let error = estimator
        .can_admit(AllOrError::Exact, &request, &capabilities, None, false)
        .expect_err("metadata is not an exact counter");
    assert_eq!(error, "exact provider/model context counter unavailable");
    assert!(
        estimator
            .can_admit(AllOrError::Exact, &request, &capabilities, Some(1), false)
            .is_ok()
    );
    let error = estimator
        .can_admit(
            AllOrError::Conservative,
            &request,
            &capabilities,
            None,
            false,
        )
        .expect_err("a provider must prove conservative bounds");
    assert_eq!(error, "conservative provider context bound unavailable");
}
