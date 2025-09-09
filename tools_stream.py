from __future__ import annotations

import rich
from pydantic import BaseModel

import openai
from openai import OpenAI


class GetWeather(BaseModel):
    city: str
    country: str


client = OpenAI(
    base_url="https://elysia.h-e.top/toolify/v1",
    api_key="sk-Rp1otEd2R2otemtfO8EOjXSvieE10EY0uXpNLx5iCSeJ2hub",
)


with client.chat.completions.stream(
    model="deepseek-v3.1",
    messages=[
        {
            "role": "user",
            "content": "What's the weather like in SF and New York?",
        },
    ],
    tools=[
        # because we're using `.parse_stream()`, the returned tool calls
        # will be automatically deserialized into this `GetWeather` type
        openai.pydantic_function_tool(GetWeather, name="get_weather"),
    ],
    parallel_tool_calls=True,
) as stream:
    for event in stream:
        print(event)
#         if event.type == "tool_calls.function.arguments.delta" or event.type == "tool_calls.function.arguments.done":
#             rich.get_console().print(event, width=80)

# print("----\n")
# rich.print(stream.get_final_completion())