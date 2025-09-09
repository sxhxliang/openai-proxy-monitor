use std::{convert, sync::OnceLock};
use std::time::Duration;

use anyhow::Result as AnyResult;
use async_trait::async_trait;
use bytes::Bytes;
use http::Uri;
use log::info;
use pingora::prelude::{ProxyHttp, Session};
use pingora_core::prelude::HttpPeer;
use pingora_error::{Error, ErrorType::HTTPStatus};
use pingora_http::{RequestHeader, ResponseHeader};
use prometheus::{register_counter_vec, register_int_counter, CounterVec, IntCounter};
use serde::{Deserialize, Deserializer};
use serde_json::from_slice;
use tiktoken_rs::CoreBPE;

use ai_api_converter::{anthropic_converter, utils::OpenAIStreamParser, AnthropicConverter, BaseConverter, ConversionResult, ConverterFactory};
use crate::rate_limiter::SlidingWindowRateLimiter;
use crate::utils::parse_request_via_path_and_header;
const USER_RESOURCE: &str = "user";

// Configurations
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

// Main gateway struct
pub struct HttpGateway<R: SlidingWindowRateLimiter + Send + Sync> {
    tokenizer: CoreBPE,
    metrics: &'static GatewayMetrics,
    peer: Peer,
    rate_limiter: R,
    rate_config: RateLimitingConfig,
}

struct Peer {
    tls: bool,
    addr: &'static str,
    port: u16,
}

// Context for request processing
pub struct Ctx {
    req_buffer: Vec<u8>,
    resp_buffer: Vec<u8>,
    openai_request: Option<OpenAIRequest>,
    user: String,
}

#[derive(Clone)]
struct OpenAIRequest {
    model: String,
    request_type: RequestType,
    prompt_tokens: u64,
}

#[derive(Clone, Debug)]
enum RequestType {
    Stream,
    NonStream,
}

// Request/Response structures
#[derive(Deserialize, Debug)]
struct OpenAIRequestBody {
    model: String,
    #[serde(default)]
    stream: bool,
    #[serde(default)]
    messages: Vec<Message>,
    #[serde(default, deserialize_with = "deserialize_prompt")]
    prompt: Option<Vec<String>>,
}

#[derive(Deserialize, Debug)]
struct Message {
    content: String,
}

#[derive(Deserialize, Debug)]
struct Usage {
    prompt_tokens: u64,
    completion_tokens: u64,
}

#[derive(Deserialize, Debug)]
struct UsageResponse {
    usage: Usage,
}

#[derive(Deserialize, Debug)]
struct StreamingResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize, Debug)]
struct Choice {
    #[serde(default)]
    delta: Option<Delta>,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Deserialize, Debug)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Deserialize, Debug)]
struct TokenUsage {
    prompt_tokens: u64,
    completion_tokens: u64,
}

// Metrics
struct GatewayMetrics {
    prompt_tokens: &'static IntCounter,
    completion_tokens: &'static IntCounter,
    total_tokens: &'static IntCounter,
    tokens_by_model: &'static CounterVec,
    tokens_by_user_model: &'static CounterVec,
}

impl GatewayMetrics {
    fn instance() -> &'static Self {
        static METRICS: OnceLock<GatewayMetrics> = OnceLock::new();
        METRICS.get_or_init(Self::init)
    }

    fn init() -> Self {
        Self {
            prompt_tokens: Box::leak(Box::new(
                register_int_counter!("prompt_tokens_total", "Prompt tokens").unwrap()
            )),
            completion_tokens: Box::leak(Box::new(
                register_int_counter!("completion_tokens_total", "Completion tokens").unwrap()
            )),
            total_tokens: Box::leak(Box::new(
                register_int_counter!("tokens_total", "Total tokens").unwrap()
            )),
            tokens_by_model: Box::leak(Box::new(
                register_counter_vec!("tokens_by_model", "Tokens by model", &["model", "type"]).unwrap()
            )),
            tokens_by_user_model: Box::leak(Box::new(
                register_counter_vec!("tokens_by_user_model", "Tokens by user and model", &["user", "model", "type"]).unwrap()
            )),
        }
    }

    fn record(&self, usage: &TokenUsage, model: &str, user: &str) {
        let total = usage.prompt_tokens + usage.completion_tokens;
        
        // Basic counters
        self.prompt_tokens.inc_by(usage.prompt_tokens);
        self.completion_tokens.inc_by(usage.completion_tokens);
        self.total_tokens.inc_by(total);

        // By model
        self.tokens_by_model.with_label_values(&[model, "prompt"]).inc_by(usage.prompt_tokens as f64);
        self.tokens_by_model.with_label_values(&[model, "completion"]).inc_by(usage.completion_tokens as f64);

        // By user and model
        self.tokens_by_user_model.with_label_values(&[user, model, "prompt"]).inc_by(usage.prompt_tokens as f64);
        self.tokens_by_user_model.with_label_values(&[user, model, "completion"]).inc_by(usage.completion_tokens as f64);
    }
}

// Deserialization helper
fn deserialize_prompt<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{self, SeqAccess, Visitor};
    use std::fmt;

    struct PromptVisitor;

    impl<'de> Visitor<'de> for PromptVisitor {
        type Value = Option<Vec<String>>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("string, array of strings, or null")
        }

        fn visit_none<E>(self) -> Result<Self::Value, E> {
            Ok(None)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
            Ok(Some(vec![value.to_string()]))
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut vec = Vec::new();
            while let Some(value) = seq.next_element()? {
                vec.push(value);
            }
            Ok(Some(vec))
        }
    }

    deserializer.deserialize_option(PromptVisitor)
}

// Implementation
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

    fn calculate_tokens(&self, text: &str) -> usize {
        self.tokenizer.encode_with_special_tokens(text).len()
    }

    fn parse_request(&self, buffer: &[u8], path: &str) -> pingora_error::Result<OpenAIRequest> {
        let body: OpenAIRequestBody = from_slice(buffer)
            .map_err(|_| Error::explain(HTTPStatus(400), "Invalid request body"))?;

        let (request_type, prompt_tokens) = if body.stream {
            let tokens = match path {
                p if p.contains("/chat/completions") => {
                    body.messages.iter()
                        .map(|msg| self.calculate_tokens(&msg.content))
                        .sum::<usize>()
                },
                p if p.contains("/completions") => {
                    body.prompt.as_ref()
                        .map(|prompts| prompts.iter().map(|p| self.calculate_tokens(p)).sum())
                        .unwrap_or(0)
                },
                _ => 0,
            };
            (RequestType::Stream, tokens as u64)
        } else {
            (RequestType::NonStream, 0)
        };

        Ok(OpenAIRequest {
            model: body.model,
            request_type,
            prompt_tokens,
        })
    }

    fn parse_streaming_response(&self, buffer: &[u8]) -> pingora_error::Result<u64> {
        let responses: Vec<StreamingResponse> = buffer
            .split(|&b| b == b'\n')
            .filter(|line| line.starts_with(b"data: {"))
            .map(|line| &line[6..])
            .filter_map(|line| from_slice(line).ok())
            .collect();
        let mut final_context = String::new();
        let completion_tokens = responses.iter()
            .flat_map(|resp| &resp.choices)
            .filter_map(|choice| {
                choice.delta.as_ref()
                    .and_then(|d| d.content.as_ref())
                    .or(choice.text.as_ref())
            })
            .map(|content| {
                final_context.push_str(content);
                self.calculate_tokens(content)
            })
            .sum::<usize>();
        // println!("Final context: \n{}", final_context);
        Ok(completion_tokens as u64)
    }

    async fn check_rate_limit(&self, user: &str) -> pingora_error::Result<()> {
        let count = self.rate_limiter
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

#[async_trait]
impl<R: SlidingWindowRateLimiter + Send + Sync> ProxyHttp for HttpGateway<R> {
    type CTX = Ctx;

    fn new_ctx(&self) -> Self::CTX {
        Ctx {
            req_buffer: Vec::with_capacity(4096),
            resp_buffer: Vec::with_capacity(8192),
            openai_request: None,
            user: String::new(),
        }
    }

    async fn upstream_peer(&self, _: &mut Session, _: &mut Self::CTX) -> pingora_error::Result<Box<HttpPeer>> {
        let peer = Box::new(HttpPeer::new(
            (self.peer.addr, self.peer.port),
            self.peer.tls,
            self.peer.addr.to_string(),
        ));
        Ok(peer)
    }

     /// Filters incoming requests
    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> pingora_error::Result<bool> {

        session
            .req_header_mut()
            .set_uri(Uri::from_static("/v1/chat/completions"));
        println!("Modified request URI to /v1/chat/completions");
        println!("Request Path: {:#?}", session.req_header().headers);
        Ok(false)
    }
    async fn request_body_filter(
        &self,
        session: &mut Session,
        body: &mut Option<Bytes>,
        end_of_stream: bool,
        ctx: &mut Self::CTX,
    ) -> pingora_error::Result<()> {
        let res = parse_request_via_path_and_header(
            session.req_header().uri.path(),
            &session.req_header().headers,
            body.as_ref().map(|b| std::str::from_utf8(b).unwrap_or(""))
        );
        println!("Parsed Request: {:?}", res);
        
        if let Some(b) = body {
            ctx.req_buffer.extend_from_slice(b);
        }

        if end_of_stream && session.req_header().method == "POST" {
            let path = session.req_header().uri.path();
            ctx.openai_request = Some(self.parse_request(&ctx.req_buffer, path)?);
            

            
            let anthropic_converter = ConverterFactory::get_converter("anthropic").unwrap();

            println!("Original request body: {}", String::from_utf8_lossy(&ctx.req_buffer));
            let json_value: serde_json::Value = serde_json::from_slice(&ctx.req_buffer)
                .map_err(|e| Error::explain(HTTPStatus(400), format!("Invalid JSON: {}", e)))?;
            let res: Result<ConversionResult, ai_api_converter::ConversionError> = anthropic_converter
                .convert_request(json_value, "openai", None).await;
            // println!("Conversion result: {:#?}", res);
            match res {
                Ok(conversion_result) => {
                    // println!("Converted request: {:?}", conversion_result.data);
                    let json_str = serde_json::to_string(&conversion_result.data.unwrap())
                        .map_err(|e| Error::explain(HTTPStatus(500), format!("JSON serialization error: {}", e)))?;
                    println!("Converted JSON string: {}", json_str);
                    
                    session
                        .req_header_mut()
                        .insert_header("Content-Type", "application/json")?;
                    session
                        .req_header_mut()
                        .insert_header("Content-Length", json_str.len().to_string())?;
                    println!("session req header: {:#?}", session.req_header().headers);
                    *body = Some(Bytes::from(json_str));
                },
                Err(_) => todo!(),
            }

        }


        Ok(())
    }

    async fn upstream_request_filter(
        &self,
        session: &mut Session,
        upstream_request: &mut RequestHeader,
        ctx: &mut Self::CTX,
    ) -> pingora_error::Result<()> {
        upstream_request.insert_header("Host", self.peer.addr)?;
        upstream_request.insert_header("Content-Type", "application/json")?;

        ctx.user = session.req_header().headers
            .get(self.rate_config.user_header_key)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        self.check_rate_limit(&ctx.user).await?;
        Ok(())
    }

    async fn response_filter(
        &self,
        _: &mut Session,
        upstream_response: &mut ResponseHeader,
        _: &mut Self::CTX,
    ) -> pingora_error::Result<()> {
        if upstream_response.status.as_u16() != 200 {
            return Err(Error::explain(
                HTTPStatus(upstream_response.status.as_u16()),
                "Upstream error",
            ));
        }
        Ok(())
    }

    fn response_body_filter(
        &self,
        _: &mut Session,
        body: &mut Option<Bytes>,
        end_of_stream: bool,
        ctx: &mut Self::CTX,
    ) -> pingora_error::Result<Option<Duration>> {
        if let Some(b) = body {
            ctx.resp_buffer.extend_from_slice(b);
            println!("Response Body: {:#?}", String::from_utf8_lossy(b).to_string());
            let data = String::from_utf8_lossy(b).to_string();
            let json_str = extract_json_from_sse(&data).unwrap();
            let json_value: serde_json::Value = serde_json::from_str(&json_str).unwrap();
            // println!("Extracted JSON: {}", json_value);

            // 你现在可以直接操作这个 Value 对象
            if let Some(model) = json_value.get("model") {
                // println!("\n提取到的 model 字段: {}", model);
                let anthropic_converter = AnthropicConverter::new();
                let _ = anthropic_converter.set_original_model(&model.to_string());
                let res = anthropic_converter.convert_from_openai_streaming_chunk(json_value);

                match res {
                    Ok(convertion_result) => {
                        println!("Converted response: {:?}", convertion_result);
                        if let Some(data) = convertion_result.data {
                            if let serde_json::Value::String(str_data) = data {
                                println!("Converted JSON string:\n{}", str_data);
                                *body = Some(Bytes::from(str_data));
                            }
                        }
                    },
                    Err(e) => {
                        eprintln!("Error during response conversion: {}", e);
                    }
                }
            }
        }


        if end_of_stream {
            if let Some(req) = &ctx.openai_request {
                let usage = match req.request_type {
                    RequestType::Stream => {
                        let completion_tokens = self.parse_streaming_response(&ctx.resp_buffer)?;
                        TokenUsage {
                            prompt_tokens: req.prompt_tokens,
                            completion_tokens,
                        }
                    },
                    RequestType::NonStream => {
                        let response: UsageResponse = from_slice(&ctx.resp_buffer)
                            .map_err(|_| Error::explain(HTTPStatus(502), "Invalid response"))?;
                        TokenUsage {
                            prompt_tokens: response.usage.prompt_tokens,
                            completion_tokens: response.usage.completion_tokens,
                        }
                    },
                };
                println!("Usage: {:?}", usage);
                // Update metrics and rate limiter
                self.metrics.record(&usage, &req.model, &ctx.user);
                let total_tokens = usage.prompt_tokens + usage.completion_tokens;
                let _ = self.rate_limiter.record_sliding_window(
                    USER_RESOURCE,
                    &ctx.user,
                    total_tokens,
                    Duration::from_secs(self.rate_config.window_duration_min * 60),
                );
            }
        }

        Ok(None)
    }

    async fn logging(&self, session: &mut Session, _: Option<&Error>, _: &mut Self::CTX) {
        let status = session.response_written()
            .map_or(0, |resp| resp.status.as_u16());
        info!("{} {} - {}", 
            session.req_header().method,
            session.req_header().uri.path(),
            status
        );
    }
}

fn extract_json_from_sse(sse_data: &str) -> Option<String> {
    // 按事件分隔符 "\n\n" 分割字符串
    for event in sse_data.split("\n\n") {
        // 查找以 "data: " 开头的行
        if let Some(line) = event.lines().find(|l| l.starts_with("data: ")) {
            // 检查是否是 [DONE] 消息
            if line.contains("[DONE]") {
                continue;
            }
            // 提取 "data: " 后面的部分，并去除首尾空格
            let json_str = line.strip_prefix("data: ").unwrap_or("");
            if !json_str.is_empty() {
                return Some(json_str.to_string());
            }
        }
    }
    None
}