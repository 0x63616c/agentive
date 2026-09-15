use crate::model::ModelTokenUsage;
use serde::{Deserialize, Serialize};

/// Token measurements reported or conservatively estimated for one scope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenUsage {
    /// Input tokens.
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cached_input: Option<u64>,
    pub reasoning: Option<u64>,
    pub provider_total: Option<u64>,
}

impl TokenUsage {
    /// Returns an explicitly unknown measurement.
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

/// Usage attributed to one model-call effect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelCallUsage {
    /// Provider/model attribution label.
    pub model: String,
    pub usage: TokenUsage,
}

/// Aggregate usage accumulated by a run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunUsage {
    /// Aggregate token measurement.
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
    /// Creates empty, complete usage.
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

    /// Merge a trusted child run exactly once into this aggregate.
    pub fn merge_child(&mut self, child: &RunUsage) {
        self.aggregate.merge(&child.aggregate);
        self.complete &= child.complete;
        self.model_calls.extend(child.model_calls.clone());
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
