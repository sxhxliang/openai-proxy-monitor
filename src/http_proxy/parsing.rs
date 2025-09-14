use serde::Deserializer;
use serde_json::from_slice;

use pingora_error::{Error, ErrorType::HTTPStatus};

use super::config::HttpGateway;
use super::types::{OpenAIRequest, OpenAIRequestBody, RequestType, StreamingResponse};

// Deserialization helper for `prompt` that accepts string or array
pub(super) fn deserialize_prompt<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{SeqAccess, Visitor};
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

impl<R> HttpGateway<R>
where
    R: crate::rate_limiter::SlidingWindowRateLimiter + Send + Sync,
{
    pub(super) fn parse_request(
        &self,
        buffer: &[u8],
        path: &str,
    ) -> pingora_error::Result<OpenAIRequest> {
        let body: OpenAIRequestBody = from_slice(buffer)
            .map_err(|_| Error::explain(HTTPStatus(400), "Invalid request body"))?;

        let (request_type, prompt_tokens) = if body.stream {
            let tokens = match path {
                p if p.contains("/chat/completions") => body
                    .messages
                    .iter()
                    .map(|msg| self.calculate_tokens(&msg.content))
                    .sum::<usize>(),
                p if p.contains("/completions") => body
                    .prompt
                    .as_ref()
                    .map(|prompts| prompts.iter().map(|p| self.calculate_tokens(p)).sum())
                    .unwrap_or(0),
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

    pub(super) fn parse_streaming_response(&self, buffer: &[u8]) -> pingora_error::Result<u64> {
        let responses: Vec<StreamingResponse> = buffer
            .split(|&b| b == b'\n')
            .filter(|line| line.starts_with(b"data: {"))
            .map(|line| &line[6..])
            .filter_map(|line| from_slice(line).ok())
            .collect();
        let mut final_context = String::new();
        let completion_tokens = responses
            .iter()
            .flat_map(|resp| &resp.choices)
            .filter_map(|choice| {
                choice
                    .delta
                    .as_ref()
                    .and_then(|d| d.content.as_ref())
                    .or(choice.text.as_ref())
            })
            .map(|content| {
                final_context.push_str(content);
                self.calculate_tokens(content)
            })
            .sum::<usize>();
        Ok(completion_tokens as u64)
    }
}

pub(super) fn extract_json_from_sse(sse_data: &str) -> Option<String> {
    for event in sse_data.split("\n\n") {
        if let Some(line) = event.lines().find(|l| l.starts_with("data: ")) {
            if line.contains("[DONE]") {
                continue;
            }
            let json_str = line.strip_prefix("data: ").unwrap_or("");
            if !json_str.is_empty() {
                return Some(json_str.to_string());
            }
        }
    }
    None
}
