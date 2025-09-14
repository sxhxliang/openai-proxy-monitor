use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http::Uri;
use log::info;
use pingora::prelude::{ProxyHttp, Session};
use pingora_core::prelude::HttpPeer;
use pingora_error::{Error, ErrorType::HTTPStatus};
use pingora_http::{RequestHeader, ResponseHeader};
use serde_json::from_slice;
use std::str::FromStr;

use ai_api_converter::{AnthropicConverter, BaseConverter, ConverterFactory, GeminiConverter, OpenAIConverter};

use crate::utils::parse_request_via_path_and_header;

use super::config::HttpGateway;
use super::parsing::extract_json_from_sse;
use super::types::{Ctx, RequestType, TokenUsage, UsageResponse};

#[async_trait]
impl<R> ProxyHttp for HttpGateway<R>
where
    R: crate::rate_limiter::SlidingWindowRateLimiter + Send + Sync,
{
    type CTX = Ctx;

    fn new_ctx(&self) -> Self::CTX {
        Ctx {
            req_buffer: Vec::with_capacity(4096),
            resp_buffer: Vec::with_capacity(8192),
            openai_request: None,
            user: String::new(),
            api_service: None,
            upstream_service: None,
            selected_peer: None,
            selected_channel: None,
            api_key_hash: None,
            routing_attempts: 0,
            fallback_used: false,
        }
    }

    async fn request_filter(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> pingora_error::Result<bool> {
        session.enable_retry_buffering();
        let request_body = session.read_request_body().await?;

        // 解析请求格式
        let res = parse_request_via_path_and_header(
            session.req_header().uri.path(),
            &session.req_header().headers,
            request_body
                .as_ref()
                .map(|b| std::str::from_utf8(b).unwrap_or("")),
        );
        println!("Parsed Request: {:?}", res);
        ctx.api_service = Some(res.service.clone());

        // 智能路由选择
        if let Some(ref api_key) = res.api_key {
            ctx.api_key_hash = Some(self.hash_api_key(api_key));
            if let Some(channel_config) =
                self.smart_route_selection(Some(api_key), res.model.as_deref())
            {
                ctx.selected_peer = Some(channel_config.peer.clone());
                ctx.upstream_service = Some(channel_config.service.clone());
                ctx.selected_channel = Some(channel_config.channel_id.clone());
                println!("Selected channel: {}", channel_config.name);
            }
        }

        // 回退到传统路由
        if ctx.selected_peer.is_none() {
            if let Some(model) = &res.model {
                if let Some(rule) = self
                    .routing
                    .iter()
                    .find(|r| model.starts_with(r.model_prefix))
                {
                    ctx.selected_peer = Some(rule.peer.clone());
                    ctx.upstream_service = Some(rule.upstream_service.clone());
                }
            }
        }

        // 最终回退到默认配置
        if ctx.selected_peer.is_none() {
            ctx.selected_peer = Some(self.peer.clone());
            ctx.upstream_service = Some(crate::utils::ApiService::OpenAI);
        }

        // 配置上游请求
        if let Some(api_key) = res.api_key {
            let upstream_service = ctx.upstream_service.as_ref().unwrap();
            self.configure_upstream_request(session, &api_key, upstream_service, &res.model)?;
        } else {
            return Err(Error::explain(HTTPStatus(401), "Missing API key"));
        }

        println!(
            "Request configured for upstream: {:?}",
            ctx.upstream_service
        );
        Ok(false)
    }


    async fn upstream_peer(
        &self,
        _: &mut Session,
        ctx: &mut Self::CTX,
    ) -> pingora_error::Result<Box<HttpPeer>> {
        // 优先使用智能路由选择的peer，回退到默认peer
        let selected = ctx
            .selected_peer
            .clone()
            .unwrap_or_else(|| self.peer.clone());
        let peer = Box::new(HttpPeer::new(
            (selected.addr, selected.port),
            selected.tls,
            selected.addr.to_string(),
        ));
        Ok(peer)
    }


    async fn upstream_request_filter(
        &self,
        session: &mut Session,
        upstream_request: &mut RequestHeader,
        ctx: &mut Self::CTX,
    ) -> pingora_error::Result<()> {
        let host = ctx
            .selected_peer
            .as_ref()
            .map(|p| p.addr)
            .unwrap_or(self.peer.addr);
        upstream_request.insert_header("Host", host)?;
        upstream_request.insert_header("Content-Type", "application/json")?;

        ctx.user = session
            .req_header()
            .headers
            .get(self.rate_config.user_header_key)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        self.check_rate_limit(&ctx.user).await?;
        Ok(())
    }


    async fn request_body_filter(
        &self,
        session: &mut Session,
        body: &mut Option<Bytes>,
        end_of_stream: bool,
        ctx: &mut Self::CTX,
    ) -> pingora_error::Result<()> {
        if let Some(b) = body {
            ctx.req_buffer.extend_from_slice(b);
        }

        if end_of_stream && session.req_header().method == "POST" {
            let path = session.req_header().uri.path();
            ctx.openai_request = Some(self.parse_request(&ctx.req_buffer, path)?);

            let source_service = ctx
                .api_service
                .clone()
                .unwrap_or(crate::utils::ApiService::Unknown);
            let target_service = ctx
                .upstream_service
                .clone()
                .unwrap_or(crate::utils::ApiService::OpenAI);

            // 如果源服务和目标服务相同，直接透传
            if source_service == target_service {
                println!("Source and target services are the same, passing through");
                return Ok(());
            }

            // 执行格式转换
            if let Ok(converted_data) =
                self.convert_request_format(&ctx.req_buffer, &source_service, &target_service)
            {
                session
                    .req_header_mut()
                    .insert_header("Content-Type", "application/json")?;
                session
                    .req_header_mut()
                    .insert_header("Content-Length", converted_data.len().to_string())?;
                *body = Some(Bytes::from(converted_data));
                println!("Request conversion completed successfully");
            }
        }

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
            let data = String::from_utf8_lossy(b).to_string();

            // 处理流式响应
            if self.is_streaming_response(&data) {
                if let Ok(Some(converted_data)) = self.convert_streaming_response(&data, ctx) {
                    *body = Some(Bytes::from(converted_data));
                }
            }
        }

        if end_of_stream {
            // 处理使用量统计
            self.finalize_usage_statistics(ctx);
        }

        Ok(None)
    }

    async fn logging(
        &self,
        session: &mut Session,
        _: Option<&pingora_error::Error>,
        _: &mut Self::CTX,
    ) {
        let status = session
            .response_written()
            .map_or(0, |resp| resp.status.as_u16());
        info!(
            "{} {} - {}",
            session.req_header().method,
            session.req_header().uri.path(),
            status
        );
    }
}

// 辅助方法实现
impl<R> HttpGateway<R>
where
    R: crate::rate_limiter::SlidingWindowRateLimiter + Send + Sync,
{
    /// 配置上游请求
    fn configure_upstream_request(
        &self,
        session: &mut Session,
        api_key: &str,
        upstream_service: &crate::utils::ApiService,
        model: &Option<String>,
    ) -> pingora_error::Result<()> {
        match upstream_service {
            crate::utils::ApiService::Anthropic => {
                session
                    .req_header_mut()
                    .insert_header("x-api-key", api_key)?;
                session
                    .req_header_mut()
                    .insert_header("anthropic-version", "2023-06-01")?;
                session
                    .req_header_mut()
                    .set_uri(Uri::from_static("/v1/messages"));
            }
            crate::utils::ApiService::OpenAI => {
                session
                    .req_header_mut()
                    .insert_header("Authorization", format!("Bearer {}", api_key))?;
                session
                    .req_header_mut()
                    .set_uri(Uri::from_static("/v1/chat/completions"));
            }
            crate::utils::ApiService::Google => {
                session
                    .req_header_mut()
                    .insert_header("x-goog-api-key", api_key)?;
                if let Some(model_name) = model {
                    let path = format!("/v1beta/models/{}:generateContent", model_name);
                    session
                        .req_header_mut()
                        .set_uri(Uri::from_str(&path).unwrap_or(Uri::from_static(
                            "/v1beta/models/gemini-pro:generateContent",
                        )));
                }
            }
            _ => {
                return Err(Error::explain(HTTPStatus(400), "Unsupported API service"));
            }
        }
        Ok(())
    }

    /// 转换请求格式
    fn convert_request_format(
        &self,
        buffer: &[u8],
        source_service: &crate::utils::ApiService,
        target_service: &crate::utils::ApiService,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let json_value: serde_json::Value = serde_json::from_slice(buffer)?;
        let source_converter = ConverterFactory::get_converter(source_service.as_str())?;

        // 执行转换
        let conversion_result = {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async {
                source_converter
                    .convert_request(json_value, target_service.as_str(), None)
                    .await
            })?
        };

        if let Some(converted_data) = conversion_result.data {
            Ok(serde_json::to_string(&converted_data)?)
        } else {
            Err("Conversion succeeded but no data returned".into())
        }
    }

    /// 判断是否为流式响应
    fn is_streaming_response(&self, data: &str) -> bool {
        data.starts_with("data: ") || data.contains("event: ") || data.contains("[DONE]")
    }

    /// 转换流式响应
    fn convert_streaming_response(
        &self,
        data: &str,
        ctx: &mut Ctx,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let json_str = match extract_json_from_sse(data) {
            Some(json) => json,
            None => return Ok(None),
        };

        let json_value: serde_json::Value = serde_json::from_str(&json_str)?;
        let model = json_value
            .get("model")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown");

        let original_service = ctx
            .api_service
            .clone()
            .unwrap_or(crate::utils::ApiService::Unknown);
        let upstream_service = ctx
            .upstream_service
            .clone()
            .unwrap_or(crate::utils::ApiService::OpenAI);

        if original_service == upstream_service {
            return Ok(None);
        }

        // 执行流式响应转换
        let conversion_result = match original_service {
            crate::utils::ApiService::Anthropic => {
                let converter = AnthropicConverter::new();
                let _ = converter.set_original_model(model);
                converter.convert_from_openai_streaming_chunk(json_value)?
            }
            crate::utils::ApiService::Google => {
                let converter = GeminiConverter::new();
                let _ = converter.set_original_model(model);
                converter.convert_from_openai_streaming_chunk(json_value)?
            }
            crate::utils::ApiService::OpenAI => {
                let converter = OpenAIConverter::new();
                let _ = converter.set_original_model(model);
                converter.convert_from_openai_streaming_chunk(json_value)?
            }
            _ => return Ok(None),
        };

        if let Some(serde_json::Value::String(s)) = conversion_result.data {
            Ok(Some(s))
        } else {
            Ok(None)
        }
    }

    /// 完成使用量统计
    fn finalize_usage_statistics(&self, ctx: &mut Ctx) {
        if let Some(req) = &ctx.openai_request {
            let usage = match req.request_type {
                RequestType::Stream => {
                    let completion_tokens =
                        self.parse_streaming_response(&ctx.resp_buffer).unwrap_or(0);
                    TokenUsage {
                        prompt_tokens: req.prompt_tokens,
                        completion_tokens,
                    }
                }
                RequestType::NonStream => match from_slice::<UsageResponse>(&ctx.resp_buffer) {
                    Ok(response) => TokenUsage {
                        prompt_tokens: response.usage.prompt_tokens,
                        completion_tokens: response.usage.completion_tokens,
                    },
                    Err(_) => TokenUsage {
                        prompt_tokens: req.prompt_tokens,
                        completion_tokens: 0,
                    },
                },
            };

            println!("Usage: {:?}", usage);
            self.metrics.record(&usage, &req.model, &ctx.user);

            let total_tokens = usage.prompt_tokens + usage.completion_tokens;
            let _ = self.rate_limiter.record_sliding_window(
                super::types::USER_RESOURCE,
                &ctx.user,
                total_tokens,
                Duration::from_secs(self.rate_config.window_duration_min * 60),
            );
        }
    }
}
