#![feature(duration_constructors, duration_constructors_lite)]

use clap::Parser;
use pingora::prelude::*;
use tiktoken_rs::cl100k_base;

use http_proxy::{HttpGateway, HttpGatewayConfig};
use crate::http_proxy::{OpenAIConfig, RateLimitingConfig};
use crate::rate_limiter::SlidingWindowRateLimiterEnum;

mod http_proxy;
mod rate_limiter;
mod utils;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    // OpenAI configuration
    #[arg(long, help = "Enable TLS for OpenAI endpoints", default_value_t = true, env)]
    openai_tls: bool,
    
    #[arg(long, help = "OpenAI endpoint port", default_value_t = 443, env)]
    openai_port: u16,
    
    #[arg(long, help = "OpenAI endpoint domain", default_value = "api.openai.com", env)]
    openai_domain: String,

    // Proxy configuration
    #[arg(long, help = "HTTP proxy port", default_value = "8080", env)]
    proxy_port: String,
    
    #[arg(long, help = "Metrics port", default_value = "9090", env)]
    metrics_port: String,

    // Rate limiting configuration  
    #[arg(long, help = "Enable rate limiting", default_value_t = false, env)]
    enable_rate_limiting: bool,
    
    #[arg(long, help = "Redis connection string", default_value = "redis://127.0.0.1:6379/0", env)]
    redis_url: String,
    
    #[arg(long, help = "Redis pool size", default_value_t = 5, env)]
    redis_pool_size: usize,
    
    #[arg(long, help = "Rate limit window (minutes)", default_value_t = 60, env)]
    rate_limit_window_min: u64,
    
    #[arg(long, help = "Max tokens per window", default_value_t = 1000, env)]
    max_tokens: u64,
    
    #[arg(long, help = "User header key", default_value = "user", env)]
    user_header: String,
}

impl Args {
    fn create_openai_config(&self) -> OpenAIConfig {
        OpenAIConfig {
            tls: self.openai_tls,
            port: self.openai_port,
            domain: self.openai_domain.clone().leak(),
        }
    }

    fn create_rate_limiting_config(&self) -> RateLimitingConfig {
        RateLimitingConfig {
            window_duration_min: self.rate_limit_window_min,
            max_prompt_tokens: self.max_tokens,
            user_header_key: self.user_header.clone().leak(),
        }
    }

    fn create_rate_limiter(&self) -> SlidingWindowRateLimiterEnum {
        if self.enable_rate_limiting {
            // TODO: Create Redis rate limiter when implemented
            SlidingWindowRateLimiterEnum::Dummy(rate_limiter::DummySlidingWindowRateLimiter {})
        } else {
            SlidingWindowRateLimiterEnum::Dummy(rate_limiter::DummySlidingWindowRateLimiter {})
        }
    }
}

fn create_gateway(args: &Args) -> anyhow::Result<HttpGateway<SlidingWindowRateLimiterEnum>> {
    let tokenizer = cl100k_base().map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))?;
    
    let config = HttpGatewayConfig {
        openai_config: args.create_openai_config(),
        tokenizer,
        sliding_window_rate_limiter: args.create_rate_limiter(),
        rate_limiting_config: args.create_rate_limiting_config(),
    };

    HttpGateway::new(config)
}

fn setup_services(server: &mut Server, args: &Args) -> anyhow::Result<()> {
    // Create and configure HTTP proxy service
    let gateway = create_gateway(args)?;
    let mut proxy_service = http_proxy_service(&server.configuration, gateway);
    proxy_service.add_tcp(&format!("0.0.0.0:{}", args.proxy_port));
    server.add_service(proxy_service);

    // Create and configure metrics service
    let mut metrics_service = pingora_core::services::listening::Service::prometheus_http_service();
    metrics_service.add_tcp(&format!("0.0.0.0:{}", args.metrics_port));
    server.add_service(metrics_service);

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    
    // Initialize logging
    env_logger::init();

    // Create and bootstrap server
    let mut server = Server::new(None)?;
    server.bootstrap();

    // Setup services
    setup_services(&mut server, &args)?;

    // Start server
    server.run_forever();
}