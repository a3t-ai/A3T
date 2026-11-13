"""The wrapper as FlowR's agent loop uses it, with a fake Claude in place of the CLI.

    cd claude-wrapper/wrapper && python -m pytest -q tests   (or: python tests/test_main.py)
"""
import asyncio
import json
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from claude_agent_sdk import AssistantMessage, PermissionResultDeny, ResultMessage, TextBlock, ToolUseBlock  # noqa: E402
from fastapi.testclient import TestClient  # noqa: E402

import src.main as w  # noqa: E402

TOOLS = [  # as FlowR's call_llm sends them
    {"type": "function", "function": {"name": "list_agents", "description": "List agents", "parameters": {"type": "object"}}},
    {"type": "function", "function": {"name": "get_agent", "description": "One agent", "parameters": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]}}},
]


class FakeClient:
    """Records the options; answers with what the test scripted."""
    seen: list = []
    script: list = []

    def __init__(self, options):
        self.options = options
        FakeClient.seen.append(options)

    async def __aenter__(self):
        return self

    async def __aexit__(self, *a):
        return False

    async def query(self, prompt):
        self.prompt = prompt

    async def receive_response(self):
        for m in FakeClient.script:
            # A tool use goes through can_use_tool first, as the CLI does.
            if isinstance(m, AssistantMessage):
                for b in m.content:
                    if isinstance(b, ToolUseBlock):
                        verdict = await self.options.can_use_tool(b.name, b.input, None)
                        assert isinstance(verdict, PermissionResultDeny) and verdict.interrupt
            yield m


def _result(text=""):
    return ResultMessage(subtype="success", duration_ms=1, duration_api_ms=1, is_error=False, num_turns=1, session_id="s",
                         result=text, usage={"input_tokens": 10, "output_tokens": 5})


def _assistant(*blocks):
    return AssistantMessage(content=list(blocks), model="sonnet")


def run():
    w.ClaudeSDKClient = FakeClient
    client = TestClient(w.app)
    ok = 0

    # 1. Rendering: tool calls and results carry over as text, by name.
    system, prompt = w.render([
        {"role": "system", "content": "You are the A3T assistant."},
        {"role": "user", "content": "How many agents?"},
        {"role": "assistant", "content": "", "tool_calls": [{"id": "c1", "type": "function", "function": {"name": "list_agents", "arguments": "{}"}}]},
        {"role": "tool", "tool_call_id": "c1", "content": '[{"id":"a1"},{"id":"a2"}]'},
        {"role": "user", "content": ""},
    ])
    assert system == "You are the A3T assistant."
    assert "Assistant called tool list_agents with {}" in prompt and 'Result of list_agents: [{"id":"a1"}' in prompt, prompt
    ok += 1

    # 2. The scope comes after the agent's own instructions, and stays when there are none.
    assert w.system_prompt("Be brief.").startswith("Be brief.") and "only questions about A3T" in w.system_prompt("Be brief.")
    assert "only questions about A3T" in w.system_prompt(None)
    ok += 1

    # 3. A schema without properties still gets them (the SDK would read its keys as parameters).
    specs = w.tool_specs(TOOLS)
    assert specs[0]["schema"] == {"type": "object", "properties": {}} and specs[1]["schema"]["required"] == ["id"]
    ok += 1

    # 4. Claude calls a tool → OpenAI tool_calls, nothing run here; the options lock Claude down.
    FakeClient.seen, FakeClient.script = [], [
        _assistant(TextBlock(text="Let me look."), ToolUseBlock(id="toolu_1", name="mcp__a3t__get_agent", input={"id": "a1"})),
        _result(),
    ]
    r = client.post("/v1/chat/completions", json={"model": "claude-opus-4-6", "tools": TOOLS,
                    "messages": [{"role": "system", "content": "Be brief."}, {"role": "user", "content": "Show agent a1"}]})
    assert r.status_code == 200, r.text
    ch = r.json()["choices"][0]
    assert ch["finish_reason"] == "tool_calls"
    tc = ch["message"]["tool_calls"][0]
    assert tc["id"] == "toolu_1" and tc["function"]["name"] == "get_agent" and json.loads(tc["function"]["arguments"]) == {"id": "a1"}
    o = FakeClient.seen[0]
    assert o.tools == [] and o.strict_mcp_config is True and o.setting_sources == [] and o.permission_mode == "default"
    assert list(o.mcp_servers) == ["a3t"] and o.model == "claude-opus-4-6" and "only questions about A3T" in o.system_prompt
    ok += 1

    # 5. FlowR parses arguments the way call_llm does (a JSON string → object).
    assert json.loads(tc["function"]["arguments"]) == {"id": "a1"}
    ok += 1

    # 6. A plain answer: finish_reason stop, no tool_calls; no tools → no MCP server at all.
    FakeClient.seen, FakeClient.script = [], [_assistant(TextBlock(text="A3T runs agents.")), _result("A3T runs agents.")]
    r = client.post("/v1/chat/completions", json={"messages": [{"role": "user", "content": "What is A3T?"}]})
    ch = r.json()["choices"][0]
    assert ch["finish_reason"] == "stop" and ch["message"]["content"] == "A3T runs agents." and "tool_calls" not in ch["message"]
    assert FakeClient.seen[0].mcp_servers == {} and FakeClient.seen[0].tools == []
    ok += 1

    # 7. Streaming carries the tool calls too.
    FakeClient.script = [_assistant(ToolUseBlock(id="toolu_2", name="mcp__a3t__list_agents", input={})), _result()]
    r = client.post("/v1/chat/completions", json={"stream": True, "tools": TOOLS, "messages": [{"role": "user", "content": "list"}]})
    chunks = [json.loads(line[6:]) for line in r.text.splitlines() if line.startswith("data: {")]
    assert chunks[0]["choices"][0]["delta"]["tool_calls"][0]["function"]["name"] == "list_agents"
    assert chunks[-1]["choices"][0]["finish_reason"] == "tool_calls"
    ok += 1

    # 8. A failed turn (not logged in) is a 502, not an answer.
    FakeClient.script = [AssistantMessage(content=[TextBlock(text="Not logged in")], model="sonnet", error="authentication_failed"),
                         ResultMessage(subtype="success", duration_ms=1, duration_api_ms=1, is_error=True, num_turns=1, session_id="s", result="Not logged in · Please run /login")]
    r = client.post("/v1/chat/completions", json={"messages": [{"role": "user", "content": "hi"}]})
    assert r.status_code == 502 and "authentication_failed" in r.json()["detail"], r.text
    ok += 1

    # 9. Health says what it is.
    assert client.get("/health").json() == {"ok": True, "scope": "a3t", "local_tools": False}
    ok += 1
    print(f"all {ok} checks pass")


def test_wrapper():
    run()


if __name__ == "__main__":
    run()
