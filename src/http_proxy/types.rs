use serde::Deserialize;
use std::collections::HashMap;

use crate::utils::ApiService;

// Internal constant
pub(super) const USER_RESOURCE: &str = "user";

// Peer config used by upstream connector
#[derive(Debug)]
pub(super) struct Peer {
    pub(super) tls: bool,
    pub(super) addr: &'static str,
    pub(super) port: u16,
}

// Context for request processing
pub struct Ctx {
    pub(super) req_buffer: Vec<u8>,
    pub(super) resp_buffer: Vec<u8>,
    pub(super) openai_request: Option<OpenAIRequest>,
    pub(super) user: String,
    pub(super) api_service: Option<ApiService>,
    pub(super) upstream_service: Option<ApiService>,
    pub(super) selected_peer: Option<Peer>,
    pub(super) selected_channel: Option<String>, // 选中的渠道ID
    pub(super) api_key_hash: Option<String>,     // API Key的哈希值
    pub(super) routing_attempts: u32,            // 路由尝试次数
    pub(super) fallback_used: bool,              // 是否使用了备用渠道
}

#[derive(Clone)]
pub(super) struct OpenAIRequest {
    pub(super) model: String,
    pub(super) request_type: RequestType,
    pub(super) prompt_tokens: u64,
}

#[derive(Clone, Debug)]
pub(super) enum RequestType {
    Stream,
    NonStream,
}

// Request/Response structures
#[derive(Deserialize, Debug)]
pub(super) struct OpenAIRequestBody {
    pub(super) model: String,
    #[serde(default)]
    pub(super) stream: bool,
    #[serde(default)]
    pub(super) messages: Vec<Message>,
    #[serde(default, deserialize_with = "super::parsing::deserialize_prompt")]
    pub(super) prompt: Option<Vec<String>>,
}

#[derive(Deserialize, Debug)]
pub(super) struct Message {
    pub(super) content: String,
}

#[derive(Deserialize, Debug)]
pub(super) struct Usage {
    pub(super) prompt_tokens: u64,
    pub(super) completion_tokens: u64,
}

#[derive(Deserialize, Debug)]
pub(super) struct UsageResponse {
    pub(super) usage: Usage,
}

#[derive(Deserialize, Debug)]
pub(super) struct StreamingResponse {
    pub(super) choices: Vec<Choice>,
}

#[derive(Deserialize, Debug)]
pub(super) struct Choice {
    #[serde(default)]
    pub(super) delta: Option<Delta>,
    #[serde(default)]
    pub(super) text: Option<String>,
}

#[derive(Deserialize, Debug)]
pub(super) struct Delta {
    #[serde(default)]
    pub(super) content: Option<String>,
}

#[derive(Deserialize, Debug)]
pub(super) struct TokenUsage {
    pub(super) prompt_tokens: u64,
    pub(super) completion_tokens: u64,
}

// Routing table entry mapping a model prefix to an upstream peer and protocol
#[derive(Clone)]
pub(super) struct RoutingRule {
    pub(super) model_prefix: &'static str,
    pub(super) peer: Peer,
    pub(super) upstream_service: ApiService,
}

impl Clone for Peer {
    fn clone(&self) -> Self {
        Self {
            tls: self.tls,
            addr: self.addr,
            port: self.port,
        }
    }
}

// API Key到渠道的映射配置
#[derive(Clone, Debug)]
pub(super) struct ApiKeyMapping {
    pub(super) api_key_hash: String, // API Key的哈希值，用于安全匹配
    pub(super) channel_id: String,   // 渠道标识符
    pub(super) peer: Peer,           // 目标服务器配置
    pub(super) service: ApiService,  // 目标API服务类型
    pub(super) weight: u32,          // 负载均衡权重
    pub(super) enabled: bool,        // 是否启用
}

// 渠道配置，支持多个API Key映射到同一个渠道
#[derive(Clone, Debug)]
pub(super) struct ChannelConfig {
    pub(super) channel_id: String,
    pub(super) name: String,
    pub(super) peer: Peer,
    pub(super) service: ApiService,
    pub(super) weight: u32,
    pub(super) enabled: bool,
    pub(super) api_keys: Vec<String>, // 绑定到此渠道的API Key哈希列表
}

// 智能路由配置 - 扩展原有的RoutingRule
#[derive(Clone, Debug)]
pub(super) struct SmartRoutingRule {
    pub(super) rule_id: String,
    pub(super) model_patterns: Vec<String>, // 支持多个模型匹配模式
    pub(super) channels: Vec<String>,       // 候选渠道列表
    pub(super) load_balance_strategy: LoadBalanceStrategy,
    pub(super) fallback_channels: Vec<String>, // 失败时的备用渠道
    pub(super) enabled: bool,
}

// 负载均衡策略
#[derive(Clone, Debug)]
pub(super) enum LoadBalanceStrategy {
    RoundRobin,       // 轮询
    WeightedRandom,   // 加权随机
    LeastConnections, // 最少连接数
    FailoverOnly,     // 仅故障转移
}

// API Key缓存管理器
#[derive(Debug)]
pub(super) struct ApiKeyCache {
    pub(super) key_to_channel: HashMap<String, String>, // API Key hash -> Channel ID
    pub(super) channel_configs: HashMap<String, ChannelConfig>,
    pub(super) routing_rules: Vec<SmartRoutingRule>,
}
