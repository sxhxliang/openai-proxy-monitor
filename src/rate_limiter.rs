// use std::ops::DerefMut;
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
// use deadpool::managed::Pool;
// use redis::AsyncCommands;

// use crate::redis_async_pool::RedisConnectionManager;

// const KEY_PREFIX: &str = "rate_limiter";

#[async_trait]
pub trait SlidingWindowRateLimiter {
    async fn record_sliding_window(
        &self,
        resource: &str,
        subject: &str,
        tokens: u64,
        size: Duration,
    ) -> Result<u64>;

    async fn fetch_sliding_window(
        &self,
        resource: &str,
        subject: &str,
        size: Duration,
    ) -> Result<u64>;
}

// pub(crate) struct RedisSlidingWindowRateLimiter {
//     connection_pool: Pool<RedisConnectionManager>,
// }

pub struct DummySlidingWindowRateLimiter {}

#[async_trait]
impl SlidingWindowRateLimiter for DummySlidingWindowRateLimiter {
    async fn record_sliding_window(
        &self,
        _resource: &str,
        _subject: &str,
        _tokens: u64,
        _size: Duration,
    ) -> Result<u64> {
        Ok(0)
    }

    async fn fetch_sliding_window(
        &self,
        _resource: &str,
        _subject: &str,
        _size: Duration,
    ) -> Result<u64> {
        Ok(0)
    }
}

pub(crate) enum SlidingWindowRateLimiterEnum {
    // Redis(RedisSlidingWindowRateLimiter),
    Dummy(DummySlidingWindowRateLimiter),
}

#[async_trait]
impl SlidingWindowRateLimiter for SlidingWindowRateLimiterEnum {
    async fn record_sliding_window(
        &self,
        resource: &str,
        subject: &str,
        tokens: u64,
        size: Duration,
    ) -> Result<u64> {
        match self {
            // SlidingWindowRateLimiterEnum::Redis(redis) => {
            //     redis
            //         .record_sliding_window(resource, subject, tokens, size)
            //         .await
            // }
            SlidingWindowRateLimiterEnum::Dummy(dummy) => {
                dummy
                    .record_sliding_window(resource, subject, tokens, size)
                    .await
            }
        }
    }

    async fn fetch_sliding_window(
        &self,
        resource: &str,
        subject: &str,
        size: Duration,
    ) -> Result<u64> {
        match self {
            // SlidingWindowRateLimiterEnum::Redis(redis) => {
            //     redis.fetch_sliding_window(resource, subject, size).await
            // }
            SlidingWindowRateLimiterEnum::Dummy(dummy) => {
                dummy.fetch_sliding_window(resource, subject, size).await
            }
        }
    }
}
