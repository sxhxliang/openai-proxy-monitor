use std::time::Duration;

use anyhow::Result as AnyResult;
use pingora_error::{Error, ErrorType::HTTPStatus};
use tiktoken_rs::CoreBPE;

use crate::rate_limiter::SlidingWindowRateLimiter;

use super::metrics::GatewayMetrics;
use super::types::{Peer, USER_RESOURCE};

pub struct HttpGatewayConfig<R: SlidingWindowRateLimiter + Send + Sync> {
    pub openai_config: OpenAIConfig,
    pub tokenizer: CoreBPE,
    pub sliding_window_rate_limiter: R,
    pub rate_limiting_config: RateLimitingConfig,
}

pub struct RateLimitingConfig {
    pub window_duration_min: u64,
    pub max_prompt_tokens: u64,
    pub user_header_key: &'static str,
}

pub struct OpenAIConfig {
    pub tls: bool,
    pub port: u16,
    pub domain: &'static str,
}

pub struct HttpGateway<R: SlidingWindowRateLimiter + Send + Sync> {
    pub(super) tokenizer: CoreBPE,
    pub(super) metrics: &'static GatewayMetrics,
    pub(super) peer: Peer,
    pub(super) rate_limiter: R,
    pub(super) rate_config: RateLimitingConfig,
}

impl<R: SlidingWindowRateLimiter + Send + Sync> HttpGateway<R> {
    pub fn new(config: HttpGatewayConfig<R>) -> AnyResult<Self> {
        Ok(Self {
            tokenizer: config.tokenizer,
            metrics: GatewayMetrics::instance(),
            rate_limiter: config.sliding_window_rate_limiter,
            peer: Peer {
                tls: config.openai_config.tls,
                addr: config.openai_config.domain,
                port: config.openai_config.port,
            },
            rate_config: config.rate_limiting_config,
        })
    }

    pub(super) fn calculate_tokens(&self, text: &str) -> usize {
        self.tokenizer.encode_with_special_tokens(text).len()
    }

    pub(super) async fn check_rate_limit(&self, user: &str) -> pingora_error::Result<()> {
        let count = self
            .rate_limiter
            .fetch_sliding_window(
                USER_RESOURCE,
                user,
                Duration::from_secs(self.rate_config.window_duration_min * 60),
            )
            .await
            .map_err(|e| Error::explain(HTTPStatus(502), e.to_string()))?;

        if count > self.rate_config.max_prompt_tokens {
            return Err(Error::explain(HTTPStatus(429), "Rate limit exceeded"));
        }
        Ok(())
    }
}

