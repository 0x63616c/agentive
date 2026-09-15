use crate::model::ModelTokenUsage;
use serde::{Deserialize, Serialize};

/// Token measurements reported or conservatively estimated for one scope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TokenUsage {
    /// Input tokens.
    pub input: Option<u64>,
    /// Generated output tokens.
    pub output: Option<u64>,
    /// Input tokens served from a provider cache.
    pub cached_input: Option<u64>,
    /// Tokens used for model reasoning, when reported.
    pub reasoning: Option<u64>,
    /// Provider-reported total token count.
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

    /// Converts provider usage into the canonical representation.
    pub fn from_model(input: ModelTokenUsage) -> Self {
        Self {
            input: input.input,
            output: input.output,
            cached_input: input.cached_input,
            reasoning: input.reasoning,
            provider_total: input.provider_total,
        }
    }

    /// Merges another measurement, preserving unknown fields as unknown.
    pub fn merge(&mut self, rhs: &TokenUsage) {
        self.input = merge_opt(self.input, rhs.input);
        self.output = merge_opt(self.output, rhs.output);
        self.cached_input = merge_opt(self.cached_input, rhs.cached_input);
        self.reasoning = merge_opt(self.reasoning, rhs.reasoning);
        self.provider_total = merge_opt(self.provider_total, rhs.provider_total);
    }

    fn complete(&self) -> bool {
        self.input.is_some()
            && self.output.is_some()
            && self.cached_input.is_some()
            && self.reasoning.is_some()
            && self.provider_total.is_some()
    }
}

/// Usage attributed to one model-call effect.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelCallUsage {
    /// Provider/model attribution label.
    pub model: String,
    /// Token measurement for this call.
    pub usage: TokenUsage,
}

/// Aggregate usage accumulated by a run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunUsage {
    /// Aggregate token measurement.
    pub aggregate: TokenUsage,
    /// Whether every recorded measurement is complete.
    pub complete: bool,
    /// Usage entries for individual model calls.
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

    /// Records one model call and merges its reported usage into the aggregate.
    pub fn add_call(&mut self, model: impl Into<String>, usage: Option<ModelTokenUsage>) {
        match usage {
            Some(usage) => {
                let token_usage = TokenUsage::from_model(usage);
                if self.model_calls.is_empty() {
                    self.aggregate = token_usage.clone();
                } else {
                    self.aggregate.merge(&token_usage);
                }
                self.complete &= token_usage.complete();
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
        if !child.model_calls.is_empty() {
            if self.model_calls.is_empty() {
                self.aggregate = child.aggregate.clone();
            } else {
                self.aggregate.merge(&child.aggregate);
            }
        }
        self.complete &= child.complete;
        self.model_calls.extend(child.model_calls.clone());
    }
}

fn merge_opt(lhs: Option<u64>, rhs: Option<u64>) -> Option<u64> {
    match (lhs, rhs) {
        (Some(a), Some(b)) => Some(a.saturating_add(b)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{RunUsage, TokenUsage};
    use crate::ModelTokenUsage;

    #[test]
    fn merge_preserves_unknown_measurements() {
        let mut known = TokenUsage {
            input: Some(3),
            output: Some(5),
            cached_input: Some(0),
            reasoning: Some(0),
            provider_total: Some(8),
        };
        known.merge(&TokenUsage::unknown());
        assert_eq!(known, TokenUsage::unknown());
    }

    #[test]
    fn partial_calls_make_an_aggregate_incomplete() {
        let mut usage = RunUsage::new();
        usage.add_call(
            "model",
            Some(ModelTokenUsage {
                input: Some(3),
                output: None,
                cached_input: Some(0),
                reasoning: Some(0),
                provider_total: Some(3),
            }),
        );
        assert!(!usage.complete);
        assert_eq!(usage.aggregate.output, None);
    }
}
