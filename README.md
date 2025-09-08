# openai-proxy-monitor(gateway)


# Run & Config


```bash
OPENAI_TLS=true OPENAI_PORT=443 OPENAI_DOMAIN="api.openai.com" cargo run --release
```

After that you can send a request to the gateway:
```bash
curl -v -X POST http://127.0.0.1:8080/v1/chat/completions \
 -d '{"model": "gpt-4o","messages": [{"role": "system", "content":"what are the best football players all time?"}], "max_tokens": 250,"temperature": 0.1, "stream": true}' \
 -H "Authorization: Bearer <API_KEY>"
```

# Usage

Here is an example to use it with the langchain client:

```python
client = OpenAI(
        openai_api_base=http://localhost:8080, # GenAI Gateway URL
        openai_api_key=config["api_key"], # OpenAI API Key. The Gateway will forward this key to the downstream OpenAI endpoint
        model_name="gpt-4o", # Model name
        logit_bias=None,
        default_headers={"user": "user1"}, # User header key which the rate limiter will use to enforce rate limiting per total tokens
    )
```

curl -v -X POST http://127.0.0.1:8080/v1/chat/completions \
     --header "x-api-key: $ANTHROPIC_API_KEY" \
     --header "anthropic-version: 2023-06-01" \
     --header "content-type: application/json" \
     --data \
'{
    "model": "qwen2.5:1.5b",
    "max_tokens": 1024,
    "stream": true,
    "messages": [
        {"role": "user", "content": "Hello, world"}
    ]
}'

curl -v -X POST http://127.0.0.1:8080/v1/chat/completions \                           ─╯
 -d '{
    "model": "qwen2.5:1.5b",
    "messages": [{"role": "system", "content":"what are the best football players all time?"}],
    "max_tokens": 250,
    "temperature": 0.1,
    "stream": true
}'

curl -v -X POST http://127.0.0.1:8080/v1/chat/completions \                           ─╯
 -d '{
    "max_tokens":1024,
    "messages":[{"content":"Hello, world","role":"user"}],
    "model":"qwen2.5:1.5b",
    "stream":true
}'


curl -v http://127.0.0.1:8080/v1/chat/completions2 -d '{"max_tokens":1024,"messages":[{"content":"Hello, world","role":"user"}],"model":"qwen2.5:1.5b","stream":true}'


curl -v -X POST http://127.0.0.1:8080/v1/chat/completions \
     --header "x-api-key: $ANTHROPIC_API_KEY" \
     --header "anthropic-version: 2023-06-01" \
     --header "content-type: application/json" \
     --data '{"model":"qwen2.5:1.5b","max_tokens":1024,"stream":true,"messages":[{"role":"user","content":"Hello, world"}]}'