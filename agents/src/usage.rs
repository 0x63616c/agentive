use crate::model::ModelTokenUsage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cached_input: Option<u64>,
    pub reasoning: Option<u64>,
    pub provider_total: Option<u64>,
}

impl TokenUsage {
    pub fn unknown() -> Self {
        Self {
            input: None,
            output: None,
            cached_input: None,
            reasoning: None,
            provider_total: None,
        }
    }

    pub fn from_model(input: ModelTokenUsage) -> Self {
        Self {
            input: input.input,
            output: input.output,
            cached_input: input.cached_input,
            reasoning: input.reasoning,
            provider_total: input.provider_total,
        }
    }

    pub fn merge(&mut self, rhs: &TokenUsage) {
        self.input = merge_opt(self.input, rhs.input);
        self.output = merge_opt(self.output, rhs.output);
        self.cached_input = merge_opt(self.cached_input, rhs.cached_input);
        self.reasoning = merge_opt(self.reasoning, rhs.reasoning);
        self.provider_total = merge_opt(self.provider_total, rhs.provider_total);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelCallUsage {
    pub model: String,
    pub usage: TokenUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunUsage {
    pub aggregate: TokenUsage,
    pub complete: bool,
    pub model_calls: Vec<ModelCallUsage>,
}

impl Default for RunUsage {
    fn default() -> Self {
        Self::new()
    }
}

impl RunUsage {
    pub fn new() -> Self {
        Self {
            aggregate: TokenUsage::unknown(),
            complete: true,
            model_calls: Vec::new(),
        }
    }

    pub fn add_call(&mut self, model: impl Into<String>, usage: Option<ModelTokenUsage>) {
        match usage {
            Some(usage) => {
                let token_usage = TokenUsage::from_model(usage);
                self.aggregate.merge(&token_usage);
                self.model_calls.push(ModelCallUsage {
                    model: model.into(),
                    usage: token_usage,
                });
            }
            None => {
                self.complete = false;
                self.model_calls.push(ModelCallUsage {
                    model: model.into(),
                    usage: TokenUsage::unknown(),
                });
            }
        }
    }
}

fn merge_opt(lhs: Option<u64>, rhs: Option<u64>) -> Option<u64> {
    match (lhs, rhs) {
        (Some(a), Some(b)) => Some(a.saturating_add(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}
