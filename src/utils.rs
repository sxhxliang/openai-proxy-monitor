use http::{HeaderMap, header::HeaderValue}; // http::HeaderMap<HeaderValue>
use serde_json::Value;

// 定义API服务枚举
#[derive(Debug, PartialEq)]
pub enum ApiService {
    Google,
    OpenAI,
    Anthropic,
    Unknown,
}

// 定义存储解析结果的结构体
#[derive(Debug)]
pub struct ApiRequest {
    pub service: ApiService,
    pub api_key: Option<String>,
    pub model: Option<String>,
}

impl ApiRequest {
    // 构造函数
    fn new(service: ApiService, api_key: Option<String>, model: Option<String>) -> Self {
        Self {
            service,
            api_key,
            model,
        }
    }
}

/// 解析URL Path和Header以识别API服务并提取信息
///
/// # Arguments
///
/// * `path` - 请求的路径字符串 (例如, "/v1/chat/completions")
/// * `headers` - http::HeaderMap<HeaderValue> 类型的请求头
/// * `body` - 请求体的可选JSON字符串
///
/// # Returns
///
/// * `ApiRequest` 结构体
pub fn parse_request_via_path_and_header(
    path: &str,
    headers: &HeaderMap<HeaderValue>,
    body: Option<&str>,
) -> ApiRequest {
    // 优先检查最独特的Header
    if let Some(api_key) = headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
    {
        let model = extract_model_from_body(body);
        return ApiRequest::new(ApiService::Anthropic, Some(api_key), model);
    }

    if let Some(api_key) = headers
        .get("x-goog-api-key")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
    {
        let model = extract_model_from_path_or_body(path, body);
        return ApiRequest::new(ApiService::Google, Some(api_key), model);
    }

    // 如果独特Header不存在，则根据Path进行分析
    // Google Path 特征: /models/gemini-pro:generateContent
    if path.contains(":generateContent") || path.contains(":embedContent") || path.contains(":batchEmbedContents") {
        let api_key = extract_bearer_token(headers);
        let model = extract_model_from_path_or_body(path, body);
        return ApiRequest::new(ApiService::Google, api_key, model);
    }

    // OpenAI Path 特征
    if path.contains("/v1/chat/completions") || path.contains("/v1/completions") {
        let api_key = extract_bearer_token(headers);
        let model = extract_model_from_body(body);
        return ApiRequest::new(ApiService::OpenAI, api_key, model);
    }
    
    // Anthropic Path 特征 (作为备用)
    if path.contains("/v1/messages") {
         // 此时 x-api-key 不存在，但路径匹配
        let api_key = extract_bearer_token(headers); 
        let model = extract_model_from_body(body);
        return ApiRequest::new(ApiService::Anthropic, api_key, model);
    }

    // 无法识别
    ApiRequest::new(ApiService::Unknown, None, None)
}

/// 辅助函数: 从请求体中提取 'model' 字段
fn extract_model_from_body(body: Option<&str>) -> Option<String> {
    body.and_then(|body_str| {
        serde_json::from_str::<Value>(body_str)
            .ok()
            .and_then(|json| {
                json.get("model")
                    .and_then(|v| v.as_str().map(String::from))
            })
    })
}

/// 辅助函数: 从Google的Path或请求体中提取 'model'
fn extract_model_from_path_or_body(path: &str, body: Option<&str>) -> Option<String> {
    // 优先从body中获取
    if let Some(model) = extract_model_from_body(body) {
        return Some(model);
    }
    // 否则尝试从path中获取
    // 路径格式: /v1beta/models/gemini-1.5-flash:generateContent
    let segments: Vec<&str> = path.split('/').collect();
    if let Some(models_pos) = segments.iter().position(|&s| s == "models") {
        if let Some(model_segment) = segments.get(models_pos + 1) {
            return model_segment.split(':').next().map(String::from);
        }
    }
    None
}

/// 辅助函数: 提取 Authorization Header 中的 Bearer Token
fn extract_bearer_token(headers: &HeaderMap<HeaderValue>) -> Option<String> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer ").map(String::from))
}

// // 示例用法
// fn main() {
//     // --- Anthropic 示例 (通过独特Header识别) ---
//     let mut anthropic_headers = HeaderMap::new();
//     anthropic_headers.insert("x-api-key", "YOUR_ANTHROPIC_API_KEY".parse().unwrap());
//     let anthropic_path = "/v1/messages";
//     let anthropic_body = r#"{"model": "claude-3-opus-20240229", "messages": []}"#;
//     let anthropic_request =
//         parse_request_via_path_and_header(anthropic_path, &anthropic_headers, Some(anthropic_body));
//     println!("--- Anthropic Request (via Header) ---");
//     println!("Path: {}", anthropic_path);
//     println!("{:?}\n", anthropic_request);

//     // --- Google Gemini 示例 (通过Path识别) ---
//     let mut gemini_headers = HeaderMap::new();
//     gemini_headers.insert("authorization", "Bearer YOUR_GEMINI_API_KEY".parse().unwrap());
//     let gemini_path = "/v1beta/models/gemini-1.5-flash:generateContent";
//     let gemini_request = parse_request_via_path_and_header(gemini_path, &gemini_headers, None);
//     println!("--- Google Gemini Request (via Path) ---");
//     println!("Path: {}", gemini_path);
//     println!("{:?}\n", gemini_request);

//     // --- Google Gemini 示例 (通过独特Header识别) ---
//     let mut gemini_headers_alt = HeaderMap::new();
//     gemini_headers_alt.insert("x-goog-api-key", "YOUR_GEMINI_API_KEY_2".parse().unwrap());
//     let gemini_path_alt = "/v1/models/gemini-pro/generateContent"; // Path不含冒号
//     let gemini_request_alt =
//         parse_request_via_path_and_header(gemini_path_alt, &gemini_headers_alt, None);
//     println!("--- Google Gemini Request (via Header) ---");
//     println!("Path: {}", gemini_path_alt);
//     println!("{:?}\n", gemini_request_alt);

//     // --- OpenAI 示例 (通过Path识别) ---
//     let mut openai_headers = HeaderMap::new();
//     openai_headers.insert("authorization", "Bearer YOUR_OPENAI_API_KEY".parse().unwrap());
//     let openai_path = "/v1/chat/completions";
//     let openai_body = r#"{"model": "gpt-4o", "messages": []}"#;
//     let openai_request =
//         parse_request_via_path_and_header(openai_path, &openai_headers, Some(openai_body));
//     println!("--- OpenAI Request (via Path) ---");
//     println!("Path: {}", openai_path);
//     println!("{:?}\n", openai_request);

//     // --- 未知请求示例 ---
//     let mut unknown_headers = HeaderMap::new();
//     unknown_headers.insert("authorization", "Bearer some_other_key".parse().unwrap());
//     let unknown_path = "/api/v2/process";
//     let unknown_request = parse_request_via_path_and_header(unknown_path, &unknown_headers, None);
//     println!("--- Unknown Request ---");
//     println!("Path: {}", unknown_path);
//     println!("{:?}", unknown_request);
// }