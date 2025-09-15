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
    api_key="SMNET-STUDIO",
)

# with client.messages.stream(
#     # thinking={
#     #     "type": "enabled",
#     #     "budget_tokens": 16000
#     # },
#     max_tokens=1024,
#     messages=[{"role": "user", "content": "Hello"}],
#     model="glm-4.5-air",
# ) as stream:
#   for text in stream.text_stream:
#       print(text, end="", flush=True)
      

message = client.messages.create(
    model="glm-4.5-air",
    max_tokens=1024,
    messages=[
        {"role": "user", "content": "Hello, Claude"},
        {"role": "assistant", "content": "Hello!"},
        {"role": "user", "content": "Can you describe LLMs to me?"}
    ],
)
print(message)
