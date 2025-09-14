use std::sync::OnceLock;

use prometheus::{CounterVec, IntCounter, register_counter_vec, register_int_counter};

use super::types::TokenUsage;

pub(super) struct GatewayMetrics {
    pub(super) prompt_tokens: &'static IntCounter,
    pub(super) completion_tokens: &'static IntCounter,
    pub(super) total_tokens: &'static IntCounter,
    pub(super) tokens_by_model: &'static CounterVec,
    pub(super) tokens_by_user_model: &'static CounterVec,
}

impl GatewayMetrics {
    pub(super) fn instance() -> &'static Self {
        static METRICS: OnceLock<GatewayMetrics> = OnceLock::new();
        METRICS.get_or_init(Self::init)
    }

    fn init() -> Self {
        Self {
            prompt_tokens: Box::leak(Box::new(
                register_int_counter!("prompt_tokens_total", "Prompt tokens").unwrap(),
            )),
            completion_tokens: Box::leak(Box::new(
                register_int_counter!("completion_tokens_total", "Completion tokens").unwrap(),
            )),
            total_tokens: Box::leak(Box::new(
                register_int_counter!("tokens_total", "Total tokens").unwrap(),
            )),
            tokens_by_model: Box::leak(Box::new(
                register_counter_vec!("tokens_by_model", "Tokens by model", &["model", "type"])
                    .unwrap(),
            )),
            tokens_by_user_model: Box::leak(Box::new(
                register_counter_vec!(
                    "tokens_by_user_model",
                    "Tokens by user and model",
                    &["user", "model", "type"]
                )
                .unwrap(),
            )),
        }
    }

    pub(super) fn record(&self, usage: &TokenUsage, model: &str, user: &str) {
        let total = usage.prompt_tokens + usage.completion_tokens;

        self.prompt_tokens.inc_by(usage.prompt_tokens);
        self.completion_tokens.inc_by(usage.completion_tokens);
        self.total_tokens.inc_by(total);

        self.tokens_by_model
            .with_label_values(&[model, "prompt"])
            .inc_by(usage.prompt_tokens as f64);
        self.tokens_by_model
            .with_label_values(&[model, "completion"])
            .inc_by(usage.completion_tokens as f64);

        self.tokens_by_user_model
            .with_label_values(&[user, model, "prompt"])
            .inc_by(usage.prompt_tokens as f64);
        self.tokens_by_user_model
            .with_label_values(&[user, model, "completion"])
            .inc_by(usage.completion_tokens as f64);
    }
}
