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
