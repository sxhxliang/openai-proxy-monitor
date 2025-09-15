use std::time::Duration;

use anyhow::Result as AnyResult;
use pingora_error::{Error, ErrorType::HTTPStatus};
use tiktoken_rs::CoreBPE;

use crate::rate_limiter::SlidingWindowRateLimiter;

use super::metrics::GatewayMetrics;
use super::types::{
    ApiKeyCache, ChannelConfig, LoadBalanceStrategy, Peer, RoutingRule, SmartRoutingRule,
    USER_RESOURCE,
};
use rand::{Rng, thread_rng};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

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
    pub(super) routing: Vec<RoutingRule>,
    pub(super) api_key_cache: Arc<RwLock<ApiKeyCache>>, // API Key映射缓存
    pub(super) smart_routing: Vec<SmartRoutingRule>,    // 智能路由规则
    pub(super) round_robin_counter: Arc<AtomicUsize>,   // 轮询计数器
}

impl<R: SlidingWindowRateLimiter + Send + Sync> HttpGateway<R> {
    pub fn new(config: HttpGatewayConfig<R>) -> AnyResult<Self> {
        // Default peer (OpenAI) from provided config
        let default_peer = Peer {
            tls: config.openai_config.tls,
            addr: config.openai_config.domain,
            port: config.openai_config.port,
        };

        // Built-in routing table: simple prefix-based mapping
        // - gpt-*   -> OpenAI upstream
        // - claude* -> Anthropic upstream
        // - gemini* -> Google upstream
        let routing = vec![
            RoutingRule {
                model_prefix: "gpt-",
                peer: default_peer.clone(),
                upstream_service: crate::utils::ApiService::OpenAI,
            },
            RoutingRule {
                model_prefix: "claude",
                peer: Peer {
                    tls: true,
                    addr: "api.anthropic.com",
                    port: 443,
                },
                upstream_service: crate::utils::ApiService::Anthropic,
            },
            RoutingRule {
                model_prefix: "gemini",
                peer: Peer {
                    tls: true,
                    addr: "generativelanguage.googleapis.com",
                    port: 443,
                },
                upstream_service: crate::utils::ApiService::Google,
            },
        ];

        // 初始化API Key缓存
        let api_key_cache = Arc::new(RwLock::new(ApiKeyCache {
            key_to_channel: HashMap::new(),
            channel_configs: HashMap::new(),
            routing_rules: Vec::new(),
        }));

        // 初始化智能路由规则（示例配置）
        let smart_routing = vec![
            SmartRoutingRule {
                rule_id: "gpt_models".to_string(),
                model_patterns: vec!["gpt-*".to_string(), "o1-*".to_string()],
                channels: vec!["openai_primary".to_string()],
                load_balance_strategy: LoadBalanceStrategy::RoundRobin,
                fallback_channels: vec!["openai_backup".to_string()],
                enabled: true,
            },
            SmartRoutingRule {
                rule_id: "claude_models".to_string(),
                model_patterns: vec!["claude*".to_string()],
                channels: vec!["anthropic_primary".to_string()],
                load_balance_strategy: LoadBalanceStrategy::WeightedRandom,
                fallback_channels: vec!["anthropic_backup".to_string()],
                enabled: true,
            },
            SmartRoutingRule {
                rule_id: "gemini_models".to_string(),
                model_patterns: vec!["gemini*".to_string()],
                channels: vec!["google_primary".to_string()],
                load_balance_strategy: LoadBalanceStrategy::FailoverOnly,
                fallback_channels: vec!["google_backup".to_string()],
                enabled: true,
            },
        ];

        Ok(Self {
            tokenizer: config.tokenizer,
            metrics: GatewayMetrics::instance(),
            rate_limiter: config.sliding_window_rate_limiter,
            peer: default_peer,
            rate_config: config.rate_limiting_config,
            routing,
            api_key_cache,
            smart_routing,
            round_robin_counter: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub(super) fn calculate_tokens(&self, text: &str) -> usize {
        print!("{}", text);
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

    /// 计算API Key的哈希值
    pub(super) fn hash_api_key(&self, api_key: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(api_key.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// 根据API Key查找渠道配置
    pub(super) fn find_channel_by_api_key(&self, api_key: &str) -> Option<ChannelConfig> {
        let api_key_hash = self.hash_api_key(api_key);
        let cache = self.api_key_cache.read().ok()?;

        let channel_id = cache.key_to_channel.get(&api_key_hash)?;
        cache.channel_configs.get(channel_id).cloned()
    }

    /// 根据模型名称找到匹配的智能路由规则
    pub(super) fn find_smart_routing_rule(&self, model: &str) -> Option<&SmartRoutingRule> {
        self.smart_routing.iter().find(|rule| {
            rule.enabled
                && rule.model_patterns.iter().any(|pattern| {
                    if pattern.ends_with('*') {
                        model.starts_with(&pattern[..pattern.len() - 1])
                    } else {
                        model == pattern
                    }
                })
        })
    }

    /// 添加渠道配置
    pub fn add_channel_config(&self, config: ChannelConfig) -> AnyResult<()> {
        let mut cache = self
            .api_key_cache
            .write()
            .map_err(|_| anyhow::anyhow!("Failed to acquire write lock"))?;

        // 更新渠道配置
        cache
            .channel_configs
            .insert(config.channel_id.clone(), config.clone());

        // 更新API Key映射
        for api_key_hash in &config.api_keys {
            cache
                .key_to_channel
                .insert(api_key_hash.clone(), config.channel_id.clone());
        }

        Ok(())
    }

    /// 添加API Key映射
    pub fn add_api_key_mapping(&self, api_key: &str, channel_id: &str) -> AnyResult<()> {
        let api_key_hash = self.hash_api_key(api_key);
        let mut cache = self
            .api_key_cache
            .write()
            .map_err(|_| anyhow::anyhow!("Failed to acquire write lock"))?;

        // 检查渠道是否存在
        if !cache.channel_configs.contains_key(channel_id) {
            return Err(anyhow::anyhow!("Channel {} does not exist", channel_id));
        }

        // 添加映射
        cache
            .key_to_channel
            .insert(api_key_hash.clone(), channel_id.to_string());

        // 更新渠道配置中的API Key列表
        if let Some(channel_config) = cache.channel_configs.get_mut(channel_id) {
            if !channel_config.api_keys.contains(&api_key_hash) {
                channel_config.api_keys.push(api_key_hash);
            }
        }

        Ok(())
    }

    /// 移除API Key映射
    pub fn remove_api_key_mapping(&self, api_key: &str) -> AnyResult<()> {
        let api_key_hash = self.hash_api_key(api_key);
        let mut cache = self
            .api_key_cache
            .write()
            .map_err(|_| anyhow::anyhow!("Failed to acquire write lock"))?;

        // 找到要移除的渠道
        if let Some(channel_id) = cache.key_to_channel.remove(&api_key_hash) {
            // 从渠道配置中移除API Key
            if let Some(channel_config) = cache.channel_configs.get_mut(&channel_id) {
                channel_config.api_keys.retain(|key| key != &api_key_hash);
            }
        }

        Ok(())
    }

    /// 获取缓存统计信息
    pub fn get_cache_stats(&self) -> AnyResult<(usize, usize, usize)> {
        let cache = self
            .api_key_cache
            .read()
            .map_err(|_| anyhow::anyhow!("Failed to acquire read lock"))?;

        Ok((
            cache.key_to_channel.len(),
            cache.channel_configs.len(),
            cache.routing_rules.len(),
        ))
    }

    /// 根据负载均衡策略选择渠道
    pub(super) fn select_channel_by_strategy(
        &self,
        channels: &[String],
        strategy: &LoadBalanceStrategy,
    ) -> Option<String> {
        if channels.is_empty() {
            return None;
        }

        let cache = self.api_key_cache.read().ok()?;

        match strategy {
            LoadBalanceStrategy::RoundRobin => {
                let counter = self.round_robin_counter.fetch_add(1, Ordering::SeqCst);
                let index = counter % channels.len();
                Some(channels[index].clone())
            }
            LoadBalanceStrategy::WeightedRandom => {
                // 根据权重进行加权随机选择
                let total_weight: u32 = channels
                    .iter()
                    .filter_map(|id| cache.channel_configs.get(id))
                    .filter(|config| config.enabled)
                    .map(|config| config.weight)
                    .sum();

                if total_weight == 0 {
                    // 没有权重信息，回退到普通随机选择
                    let mut rng = thread_rng();
                    let index = rng.gen_range(0..channels.len());
                    return Some(channels[index].clone());
                }

                let mut rng = thread_rng();
                let target = rng.gen_range(0..total_weight);
                let mut current_weight = 0;

                for channel_id in channels {
                    if let Some(config) = cache.channel_configs.get(channel_id) {
                        if config.enabled {
                            current_weight += config.weight;
                            if current_weight > target {
                                return Some(channel_id.clone());
                            }
                        }
                    }
                }

                // 如果没有匹配，返回第一个启用的渠道
                channels
                    .iter()
                    .find(|id| {
                        cache
                            .channel_configs
                            .get(*id)
                            .map_or(false, |config| config.enabled)
                    })
                    .cloned()
            }
            LoadBalanceStrategy::LeastConnections => {
                // 简化实现：选择第一个启用的渠道
                // TODO: 实际实现需要跟踪每个渠道的连接数
                channels
                    .iter()
                    .find(|id| {
                        cache
                            .channel_configs
                            .get(*id)
                            .map_or(false, |config| config.enabled)
                    })
                    .cloned()
            }
            LoadBalanceStrategy::FailoverOnly => {
                // 故障转移模式：按顺序选择第一个可用的渠道
                channels
                    .iter()
                    .find(|id| {
                        cache
                            .channel_configs
                            .get(*id)
                            .map_or(false, |config| config.enabled)
                    })
                    .cloned()
            }
        }
    }

    /// 智能路由选择：根据API Key和模型选择最佳渠道
    pub(super) fn smart_route_selection(
        &self,
        api_key: Option<&str>,
        model: Option<&str>,
    ) -> Option<ChannelConfig> {
        // Step 1: 优先考虑API Key直接映射的渠道
        if let Some(key) = api_key {
            if let Some(channel_config) = self.find_channel_by_api_key(key) {
                if channel_config.enabled {
                    return Some(channel_config);
                }
            }
        }

        // Step 2: 根据模型名称进行智能路由
        if let Some(model_name) = model {
            if let Some(routing_rule) = self.find_smart_routing_rule(model_name) {
                // 首先尝试主要渠道
                if let Some(selected_channel_id) = self.select_channel_by_strategy(
                    &routing_rule.channels,
                    &routing_rule.load_balance_strategy,
                ) {
                    let cache = self.api_key_cache.read().ok()?;
                    if let Some(channel_config) = cache.channel_configs.get(&selected_channel_id) {
                        if channel_config.enabled {
                            return Some(channel_config.clone());
                        }
                    }
                }

                // 如果主要渠道不可用，尝试备用渠道
                if !routing_rule.fallback_channels.is_empty() {
                    if let Some(fallback_channel_id) = self.select_channel_by_strategy(
                        &routing_rule.fallback_channels,
                        &LoadBalanceStrategy::FailoverOnly,
                    ) {
                        let cache = self.api_key_cache.read().ok()?;
                        if let Some(channel_config) =
                            cache.channel_configs.get(&fallback_channel_id)
                        {
                            if channel_config.enabled {
                                return Some(channel_config.clone());
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// 检查渠道健康状态（简化版本）
    pub(super) fn check_channel_health(&self, channel_id: &str) -> bool {
        if let Ok(cache) = self.api_key_cache.read() {
            if let Some(config) = cache.channel_configs.get(channel_id) {
                return config.enabled;
            }
        }
        false
    }

    /// 启用/禁用渠道
    pub fn set_channel_enabled(&self, channel_id: &str, enabled: bool) -> AnyResult<()> {
        let mut cache = self
            .api_key_cache
            .write()
            .map_err(|_| anyhow::anyhow!("Failed to acquire write lock"))?;

        if let Some(config) = cache.channel_configs.get_mut(channel_id) {
            config.enabled = enabled;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Channel {} not found", channel_id))
        }
    }

    /// 更新渠道权重
    pub fn update_channel_weight(&self, channel_id: &str, weight: u32) -> AnyResult<()> {
        let mut cache = self
            .api_key_cache
            .write()
            .map_err(|_| anyhow::anyhow!("Failed to acquire write lock"))?;

        if let Some(config) = cache.channel_configs.get_mut(channel_id) {
            config.weight = weight;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Channel {} not found", channel_id))
        }
    }
}
