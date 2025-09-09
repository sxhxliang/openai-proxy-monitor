# import anthropic

# client = anthropic.Anthropic()

# with client.messages.stream(
#     model="claude-opus-4-1-20250805",
#     max_tokens=20000,
#     thinking={
#         "type": "enabled",
#         "budget_tokens": 16000
#     },
#     messages=[
#         {
#             "role": "user",
#             "content": "What is 27 * 453?"
#         }
#     ],
# ) as stream:
#     for event in stream:
#         if event.type == "content_block_delta":
#             if event.delta.type == "thinking_delta":
#                 print(event.delta.thinking, end="", flush=True)
#             elif event.delta.type == "text_delta":
#                 print(event.delta.text, end="", flush=True)

import anthropic

client = anthropic.Anthropic(
    base_url="http://127.0.0.1:8080",
    api_key="sk-xxxx",
)

with client.messages.stream(
    max_tokens=1024,
    messages=[{"role": "user", "content": "Hello"}],
    model="qwen2.5:1.5b",
) as stream:
  for text in stream.text_stream:
      print(text, end="", flush=True)