use bytes::Bytes;
use http::header::CONTENT_ENCODING;
use pingora::{protocols::http::compression::Algorithm, proxy::ProxyHttp};
use flate2::read::GzDecoder;
use std::io::Read;

use crate::{http_proxy::HttpGateway, rate_limiter::SlidingWindowRateLimiterEnum};

/// Decodes response body based on content-encoding header
pub fn decode_body(ctx: &<HttpGateway<SlidingWindowRateLimiterEnum> as ProxyHttp>::CTX, body: &Option<Bytes>) -> Option<Bytes> {
    match ctx.vars.get(CONTENT_ENCODING.as_str()) {
        Some(content_encoding) => {
            if let Some(b) = body {
                if content_encoding.contains("gzip") {
                    log::info!("Decompressing GZIP body {:?}",  String::from_utf8_lossy(b).to_string());

                    let mut decompressor = Algorithm::Gzip.decompressor(true).unwrap();
                    return decompressor.encode(b.as_ref(), true).ok();
                }
            }
            body.clone()
        }
        None => body.clone(),
    }
}

/// Encodes response body based on content-encoding header
pub fn encode_body(ctx: &<HttpGateway<SlidingWindowRateLimiterEnum> as ProxyHttp>::CTX, body: &Option<Bytes>) -> Option<Bytes> {
    match ctx.vars.get(CONTENT_ENCODING.as_str()) {
        Some(content_encoding) => {
            if let Some(b) = body {
                if content_encoding.contains("gzip") {
                    log::debug!("Compressing GZIP body");
                    let mut compressor = Algorithm::Gzip.compressor(5).unwrap();
                    return compressor.encode(b.as_ref(), true).ok();
                }
            }
            body.clone()
        }
        None => body.clone(),
    }
}