use std::time::Duration;

use async_trait::async_trait;
use bytes::Bytes;
use http::Uri;
use std::str::FromStr;
use log::info;
use pingora::prelude::{ProxyHttp, Session};
use pingora_core::prelude::HttpPeer;
use pingora_error::{Error, ErrorType::HTTPStatus};
use pingora_http::{RequestHeader, ResponseHeader};
use serde_json::from_slice;

use ai_api_converter::{AnthropicConverter, BaseConverter, ConversionResult, ConverterFactory, GeminiConverter, OpenAIConverter};

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
            api_service: None,
            upstream_service: None,
            selected_peer: None,
        }
    }

    async fn upstream_peer(
        &self,
        _: &mut Session,
        ctx: &mut Self::CTX,
    ) -> pingora_error::Result<Box<HttpPeer>> {
        // Prefer selected peer from routing; fallback to default peer
        let selected = ctx.selected_peer.clone().unwrap_or_else(|| self.peer.clone());
        let peer = Box::new(HttpPeer::new(
            (selected.addr, selected.port),
            selected.tls,
            selected.addr.to_string(),
        ));
        Ok(peer)
    }

    async fn request_filter(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> pingora_error::Result<bool> {
        // session
        //     .req_header_mut()
        //     .set_uri(Uri::from_static("/v1/chat/completions"));
        session.enable_retry_buffering();
        let request_body = session.read_request_body().await?;

        let res = parse_request_via_path_and_header(
            session.req_header().uri.path(),
            &session.req_header().headers,
            request_body.as_ref().map(|b| std::str::from_utf8(b).unwrap_or("")),
        );
        println!("Parsed Request: {:?}", res);
        ctx.api_service = Some(res.service.clone());

        // Choose upstream by model using routing rules
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

        // Default to configured OpenAI peer/protocol if no routing matched
        if ctx.selected_peer.is_none() {
            ctx.selected_peer = Some(self.peer.clone());
            ctx.upstream_service = Some(crate::utils::ApiService::OpenAI);
        }

        // Set auth header suitable for upstream protocol (if api key present)
        if let Some(api_key) = res.api_key {
            match ctx.upstream_service.as_ref().unwrap() {
                crate::utils::ApiService::Anthropic => {
                    session
                        .req_header_mut()
                        .insert_header("x-api-key", api_key)?;
                    // Anthropic messages endpoint
                    session
                        .req_header_mut()
                        .set_uri(Uri::from_static("/v1/messages"));
                }
                crate::utils::ApiService::OpenAI => {
                    session.req_header_mut().insert_header(
                        "Authorization",
                        format!("Bearer {}", api_key),
                    )?;
                    session
                        .req_header_mut()
                        .set_uri(Uri::from_static("/v1/chat/completions"));
                }
                crate::utils::ApiService::Google => {
                    // Prefer x-goog-api-key; many clients also accept Bearer
                    session
                        .req_header_mut()
                        .insert_header("x-goog-api-key", api_key)?;
                    // If model known, route to generateContent for that model
                    if let Some(model) = &res.model {
                        let path = format!("/v1beta/models/{}:generateContent", model);
                        session.req_header_mut().set_uri(Uri::from_str(&path).unwrap_or(Uri::from_static("/v1beta/models/gemini-pro:generateContent")));
                    }
                }
                _ => {
                    return Err(Error::explain(HTTPStatus(400), "Unsupported API service"));
                }
            }
        }

        println!("Origin Request Path: {:#?}", session.req_header());
        Ok(false)
    }

    async fn request_body_filter(
        &self,
        session: &mut Session,
        body: &mut Option<Bytes>,
        end_of_stream: bool,
        ctx: &mut Self::CTX,
    ) -> pingora_error::Result<()> {


        println!("Converted Request Path: {:#?}", session.req_header());
        if let Some(b) = body {
            ctx.req_buffer.extend_from_slice(b);
        }

        if end_of_stream && session.req_header().method == "POST" {
            let path = session.req_header().uri.path();
            ctx.openai_request = Some(self.parse_request(&ctx.req_buffer, path)?);

            let source_converter =
                ConverterFactory::get_converter(ctx.api_service.clone().unwrap().as_str()).unwrap();
            println!("Using converter for source service: {}", source_converter.get_name());
            println!(
                "Original request body: {}",
                String::from_utf8_lossy(&ctx.req_buffer)
            );
            let json_value: serde_json::Value = serde_json::from_slice(&ctx.req_buffer)
                .map_err(|e| Error::explain(HTTPStatus(400), format!("Invalid JSON: {}", e)))?;
            // Convert from original protocol to the chosen upstream protocol
            let target = ctx
                .upstream_service
                .as_ref()
                .map(|s| s.as_str())
                .unwrap_or("openai");
            let res: Result<ConversionResult, ai_api_converter::ConversionError> =
                source_converter.convert_request(json_value, target, None).await;
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
                // Convert response chunks from upstream protocol back to client's original protocol
                let original = ctx.api_service.clone().unwrap_or(crate::utils::ApiService::Unknown);
                let upstream = ctx
                    .upstream_service
                    .clone()
                    .unwrap_or(crate::utils::ApiService::OpenAI);

                let result: Result<Option<String>, ai_api_converter::ConversionError> = match upstream {
                    // If upstream is OpenAI, we can convert to client protocol using available converters
                    crate::utils::ApiService::OpenAI => {
                        let conv_res = match original {
                            crate::utils::ApiService::Anthropic => {
                                let converter = AnthropicConverter::new();
                                let _ = converter.set_original_model(&model.to_string());
                                converter.convert_from_openai_streaming_chunk(json_value)
                            }
                            crate::utils::ApiService::OpenAI => {
                                let converter = OpenAIConverter::new();
                                let _ = converter.set_original_model(&model.to_string());
                                converter.convert_from_openai_streaming_chunk(json_value)
                            }
                            crate::utils::ApiService::Google => {
                                let converter = GeminiConverter::new();
                                let _ = converter.set_original_model(&model.to_string());
                                converter.convert_from_openai_streaming_chunk(json_value)
                            }
                            _ => Err(ai_api_converter::ConversionError::UnsupportedFormat(
                                original.as_str().to_string(),
                            )),
                        };
                        match conv_res {
                            Ok(conversion_result) => {
                                if let Some(serde_json::Value::String(s)) = conversion_result.data {
                                    Ok(Some(s))
                                } else {
                                    Ok(None)
                                }
                            }
                            Err(e) => Err(e),
                        }
                    }
                    // If upstream equals original, passthrough
                    u if u == original => Ok(None),
                    // Other upstream protocols are not yet supported for conversion
                    _ => {
                        println!(
                            "Upstream protocol {:?} to {:?} conversion not supported; passthrough",
                            upstream.as_str(),
                            original.as_str()
                        );
                        Ok(None)
                    }
                };

                match result {
                    Ok(Some(str_data)) => {
                        println!("Converted JSON string:\n{}", str_data);
                        *body = Some(Bytes::from(str_data));
                    }
                    Ok(None) => {
                        // passthrough
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
