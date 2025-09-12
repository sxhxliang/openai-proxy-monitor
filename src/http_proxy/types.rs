use serde::Deserialize;

use crate::utils::ApiService;

// Internal constant
pub(super) const USER_RESOURCE: &str = "user";

// Peer config used by upstream connector
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
