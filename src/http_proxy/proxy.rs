use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use log::info;
use pingora::prelude::{ProxyHttp, Session};
use pingora_core::prelude::HttpPeer;
use pingora_error::{Error, ErrorType::HTTPStatus};
use pingora_http::{RequestHeader, ResponseHeader};
use serde_json::from_slice;

use ai_api_converter::{AnthropicConverter, BaseConverter, ConversionResult, ConverterFactory};

use crate::utils::parse_request_via_path_and_header;

use super::config::HttpGateway;
use super::parsing::{extract_json_from_sse};
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
        }
    }

    async fn upstream_peer(
        &self,
        _: &mut Session,
        _: &mut Self::CTX,
    ) -> pingora_error::Result<Box<HttpPeer>> {
        let peer = Box::new(HttpPeer::new(
            (self.peer.addr, self.peer.port),
            self.peer.tls,
            self.peer.addr.to_string(),
        ));
        Ok(peer)
    }

    async fn request_filter(
        &self,
        session: &mut Session,
        _: &mut Self::CTX,
    ) -> pingora_error::Result<bool> {
        println!("Origin Request Path: {:#?}", session.req_header().headers);
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
            body.as_ref().map(|b| std::str::from_utf8(b).unwrap_or("")),
        );
        println!("Parsed Request: {:?}", res);

        if let Some(b) = body {
            ctx.req_buffer.extend_from_slice(b);
        }

        if end_of_stream && session.req_header().method == "POST" {
            let path = session.req_header().uri.path();
            ctx.openai_request = Some(self.parse_request(&ctx.req_buffer, path)?);

            let anthropic_converter =
                ConverterFactory::get_converter(res.service.as_str()).unwrap();

            println!(
                "Original request body: {}",
                String::from_utf8_lossy(&ctx.req_buffer)
            );
            let json_value: serde_json::Value = serde_json::from_slice(&ctx.req_buffer)
                .map_err(|e| Error::explain(HTTPStatus(400), format!("Invalid JSON: {}", e)))?;
            let res: Result<ConversionResult, ai_api_converter::ConversionError> =
                anthropic_converter
                    .convert_request(json_value, "openai", None)
                    .await;
            match res {
                Ok(conversion_result) => {
                    let json_str = serde_json::to_string(&conversion_result.data.unwrap())
                        .map_err(|e| {
                            Error::explain(
                                HTTPStatus(500),
                                format!("JSON serialization error: {}", e),
                            )
                        })?;
                    println!("Converted JSON string: {}", json_str);

                    session
                        .req_header_mut()
                        .insert_header("Content-Type", "application/json")?;
                    session
                        .req_header_mut()
                        .insert_header("Content-Length", json_str.len().to_string())?;
                    println!("session req header: {:#?}", session.req_header().headers);
                    *body = Some(Bytes::from(json_str));
                }
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
            println!(
                "Response Body: {:#?}",
                String::from_utf8_lossy(b).to_string()
            );
            let data = String::from_utf8_lossy(b).to_string();
            let json_str = extract_json_from_sse(&data).unwrap();
            let json_value: serde_json::Value = serde_json::from_str(&json_str).unwrap();

            if let Some(model) = json_value.get("model") {
                let anthropic_converter = AnthropicConverter::new();
                let _ = anthropic_converter.set_original_model(&model.to_string());
                let res = anthropic_converter.convert_from_openai_streaming_chunk(json_value);

                match res {
                    Ok(convertion_result) => {
                        println!("Converted response: {:?}", convertion_result);
                        if let Some(data) = convertion_result.data
                            && let serde_json::Value::String(str_data) = data
                        {
                            println!("Converted JSON string:\n{}", str_data);
                            *body = Some(Bytes::from(str_data));
                        }
                    }
                    Err(e) => {
                        eprintln!("Error during response conversion: {}", e);
                    }
                }
            }
        }

        if end_of_stream && let Some(req) = &ctx.openai_request {
            let usage = match req.request_type {
                RequestType::Stream => {
                    let completion_tokens = self.parse_streaming_response(&ctx.resp_buffer)?;
                    TokenUsage {
                        prompt_tokens: req.prompt_tokens,
                        completion_tokens,
                    }
                }
                RequestType::NonStream => {
                    let response: UsageResponse = from_slice(&ctx.resp_buffer)
                        .map_err(|_| Error::explain(HTTPStatus(502), "Invalid response"))?;
                    TokenUsage {
                        prompt_tokens: response.usage.prompt_tokens,
                        completion_tokens: response.usage.completion_tokens,
                    }
                }
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

        Ok(None)
    }

    async fn logging(&self, session: &mut Session, _: Option<&pingora_error::Error>, _: &mut Self::CTX) {
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

