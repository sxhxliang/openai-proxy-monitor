// use std::ops::DerefMut;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

#[cfg(test)]
mod tests {
    use std::net::{Ipv4Addr, SocketAddrV4, TcpListener};
    use std::time::Duration;

    use deadpool::managed::{Pool, PoolConfig};
    use redis::Client;
    use testcontainers::{
        core::{IntoContainerPort, WaitFor},
        runners::AsyncRunner,
        GenericImage, ImageExt,
    };

    use crate::rate_limiter;
    use crate::rate_limiter::{
        DummySlidingWindowRateLimiter, SlidingWindowRateLimiter,
        SlidingWindowRateLimiterEnum,
    };
    // use crate::redis_async_pool::RedisConnectionManager;


    #[tokio::test]
    async fn test_dummy_rate_limiter() {
        let dummy_rate_limiter = DummySlidingWindowRateLimiter {};
        let count = dummy_rate_limiter
            .record_sliding_window("user", "test-user-1", 10, Duration::from_secs(1))
            .await
            .expect("Failed to record sliding window");
        assert_eq!(count, 0);

        let count = dummy_rate_limiter
            .fetch_sliding_window("user", "test-user-1", Duration::from_secs(1))
            .await
            .expect("Failed to fetch sliding window");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_enum_dummy_rate_limiter() {
        let dummy_rate_limiter =
            SlidingWindowRateLimiterEnum::Dummy(rate_limiter::DummySlidingWindowRateLimiter {});
        let count = dummy_rate_limiter
            .record_sliding_window("user", "test-user-1", 10, Duration::from_secs(1))
            .await
            .expect("Failed to record sliding window");
        assert_eq!(count, 0);

        let count = dummy_rate_limiter
            .fetch_sliding_window("user", "test-user-1", Duration::from_secs(1))
            .await
            .expect("Failed to fetch sliding window");
        assert_eq!(count, 0);
    }

    fn find_free_port() -> u16 {
        let listener = TcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).unwrap();
        listener.local_addr().unwrap().port()
    }
}
