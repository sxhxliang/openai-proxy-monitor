curl -v http://127.0.0.1:8080/v1/chat/completions  -H "Authorization: Bearer SMNET-STUDIO" -d '{"max_tokens":1024,"messages":[{"content":"Hello, world","role":"user"}],"model":"glm-4.5-air","stream":true}'


curl http://127.0.0.1:8080/v1/messages \
     --header "x-api-key: SMNET-STUDIO" \
     --header "anthropic-version: 2023-06-01" \
     --header "content-type: application/json" \
     --data '{"max_tokens":1024,"messages":[{"content":"Hello, world","role":"user"}],"model":"glm-4.5-air","stream":true}'

     
curl http://127.0.0.1:8080/v1/messages \
     --header "x-api-key: SMNET-STUDIO" \
     --header "anthropic-version: 2023-06-01" \
     --header "content-type: application/json" \
     --data \
'{
    "model": "glm-4.5-air",
    "max_tokens": 1024,
    "stream":true,
    "messages": [
        {"role": "user", "content": "Hello, world"}
    ]
}'