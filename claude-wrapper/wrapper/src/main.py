"""Claude Code → OpenAI-compatible model for A3T.

FlowR's agent runtime (`call: a3t:agent`) runs the tool loop itself: it lists the
agent's MCP tools as the logged-in user, sends them as OpenAI `tools`, expects
`tool_calls` back, calls each tool as that user (with the agent's policies), and
sends the results in the next request. This wrapper is only the model in that loop:

  - The request's `tools` are shown to Claude as an in-process MCP server ("a3t").
    When Claude calls one, the call is NOT run here: it is returned to FlowR as an
    OpenAI `tool_call` (finish_reason "tool_calls"). FlowR runs it as the user.
    So no user token ever reaches this process, and A3T's policies always apply.
  - Claude Code's own tools are off (no shell, files, web, sub-agents), and only
    the servers given here exist (--strict-mcp-config): no claude.ai connectors,
    no .mcp.json, no settings files.
  - A3T_SCOPE (default on) adds a fixed instruction after the agent's own: answer
    only about A3T, with A3T's tools.

Auth: Claude Code reads CLAUDE_CODE_OAUTH_TOKEN (or ANTHROPIC_API_KEY) from the environment.
"""

import json
import os
import time
import uuid

from fastapi import FastAPI, HTTPException
from fastapi.responses import StreamingResponse

from claude_agent_sdk import (
    AssistantMessage,
    ClaudeAgentOptions,
    ClaudeSDKClient,
    PermissionResultDeny,
    ResultMessage,
    SdkMcpTool,
    TextBlock,
    ToolUseBlock,
    create_sdk_mcp_server,
)

app = FastAPI(title="Claude Code as an A3T model", version="0.2.0")

CWD = os.environ.get("CLAUDE_CWD", "/workspace")
DEFAULT_MODEL = os.environ.get("CLAUDE_MODEL", "sonnet")
SCOPE_ON = os.environ.get("A3T_SCOPE", "on").lower() not in ("0", "off", "false", "no")
SERVER = "a3t"                      # Claude sees the tools as mcp__a3t__<name>
PREFIX = f"mcp__{SERVER}__"
MAX_RESULT_CHARS = int(os.environ.get("A3T_TOOL_RESULT_CHARS", "20000"))

SCOPE = os.environ.get("A3T_SCOPE_PROMPT") or (
    "You work inside A3T, the platform for building, running and governing AI agents "
    "(agents, templates, flows, triggers, resources, memory, knowledge, policies, runtimes).\n"
    "- Answer only questions about A3T and the work the user does in it. For anything else, "
    "say in one sentence that you only help with A3T, and offer an A3T next step.\n"
    "- To read or change A3T, use only the tools you are given. They act as the signed-in user, "
    "with that user's permissions. Never claim a tool is missing or unauthorised unless a tool "
    "result says so; never mention connectors, local files, shells or Claude Code.\n"
    "- When no tool fits, say what A3T can and cannot do from what you know, briefly."
)


# ── OpenAI → a prompt Claude can continue ────────────────────────────────────

def _flatten(content) -> str:
    """OpenAI content: a string or a list of parts."""
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "".join(p.get("text", "") for p in content if isinstance(p, dict) and p.get("type") in (None, "text"))
    return "" if content is None else str(content)


def _call_parts(tc: dict) -> tuple[str, str, object]:
    """A tool call in OpenAI form ({id, function: {name, arguments}}) or FlowR's ({id, name, input})."""
    fn = tc.get("function") or {}
    name = fn.get("name") or tc.get("name") or "tool"
    args = fn.get("arguments", tc.get("input", {}))
    if isinstance(args, str):
        try:
            args = json.loads(args) if args.strip() else {}
        except ValueError:
            pass
    return tc.get("id") or "", name, args


def render(messages: list[dict]) -> tuple[str | None, str]:
    """System text, and the conversation as one transcript with tool calls and results.

    Every request starts a fresh Claude session, so earlier tool calls and their
    results are carried as text; Claude continues from the end of it.
    """
    system_parts, lines, names = [], [], {}
    for m in messages:
        role = m.get("role")
        text = _flatten(m.get("content")).strip()
        if role == "system":
            if text:
                system_parts.append(text)
        elif role == "user":
            if text:
                lines.append(f"User: {text}")
        elif role == "assistant":
            if text:
                lines.append(f"Assistant: {text}")
            for tc in m.get("tool_calls") or []:
                cid, name, args = _call_parts(tc)
                names[cid] = name
                lines.append(f"Assistant called tool {name} with {json.dumps(args, ensure_ascii=False)}")
        elif role == "tool":
            name = names.get(m.get("tool_call_id") or "", m.get("name") or "the tool")
            if len(text) > MAX_RESULT_CHARS:
                text = text[:MAX_RESULT_CHARS] + f"… [{len(text) - MAX_RESULT_CHARS} more characters cut]"
            lines.append(f"Result of {name}: {text or '(empty)'}")
    system = "\n\n".join(system_parts).strip() or None
    return system, "\n\n".join(lines).strip()


def system_prompt(agent_system: str | None) -> str | None:
    """The agent's own instructions, then the A3T scope (which it cannot switch off)."""
    parts = [p for p in (agent_system, SCOPE if SCOPE_ON else None) if p]
    return "\n\n".join(parts) or None


def tool_specs(tools: list[dict] | None) -> list[dict]:
    """OpenAI tools → [{name, description, schema}] (a schema always has properties)."""
    out = []
    for t in tools or []:
        fn = t.get("function") if t.get("type") == "function" else t
        if not isinstance(fn, dict) or not fn.get("name"):
            continue
        schema = fn.get("parameters") or fn.get("input_schema") or {}
        schema = {**schema, "type": "object", "properties": schema.get("properties") or {}}
        out.append({"name": fn["name"], "description": fn.get("description") or "", "schema": schema})
    return out


def tool_calls_out(calls: list[dict]) -> list[dict]:
    """Claude's tool uses → OpenAI tool_calls."""
    return [{
        "id": c["id"] or "call_" + uuid.uuid4().hex[:20],
        "type": "function",
        "function": {"name": c["name"], "arguments": json.dumps(c["input"], ensure_ascii=False)},
    } for c in calls]


def claude_model(requested: str | None) -> str | None:
    """Pass a model only when it is a Claude model or alias; A3T may send another name."""
    if requested:
        r = requested.lower()
        if r.startswith("claude") or r in ("sonnet", "opus", "haiku"):
            return requested
    return DEFAULT_MODEL or None


# ── One model turn ───────────────────────────────────────────────────────────

async def _never_runs(_args):  # the call goes back to FlowR instead (see _deny)
    return {"content": [{"type": "text", "text": "This tool runs in A3T, not here."}], "is_error": True}


async def turn(system: str | None, prompt: str, model: str | None, tools: list[dict]):
    """Claude's next step: (text, tool calls, usage). Tool calls are captured, never run."""
    calls: list[dict] = []

    async def _deny(name, _input, _ctx):
        # Every tool request ends the turn here; FlowR runs the call as the user.
        return PermissionResultDeny(message="Handed to A3T.", interrupt=True)

    servers = {}
    if tools:
        servers[SERVER] = create_sdk_mcp_server(
            name=SERVER, version="1.0.0",
            tools=[SdkMcpTool(name=t["name"], description=t["description"], input_schema=t["schema"], handler=_never_runs) for t in tools],
        )
    options = ClaudeAgentOptions(
        cwd=CWD,
        tools=[],                       # none of Claude Code's own tools
        mcp_servers=servers,
        strict_mcp_config=True,         # only these servers: no claude.ai connectors, no .mcp.json
        setting_sources=[],             # no settings or CLAUDE.md from disk
        permission_mode="default",
        can_use_tool=_deny,
        system_prompt=system_prompt(system),
        model=claude_model(model),
    )

    text, result, error = "", None, None
    async with ClaudeSDKClient(options=options) as client:
        await client.query(prompt)
        async for message in client.receive_response():
            if isinstance(message, AssistantMessage):
                error = error or getattr(message, "error", None)
                for block in message.content:
                    if isinstance(block, TextBlock):
                        text += block.text
                    elif isinstance(block, ToolUseBlock):
                        name = block.name[len(PREFIX):] if block.name.startswith(PREFIX) else block.name
                        calls.append({"id": block.id, "name": name, "input": block.input or {}})
            elif isinstance(message, ResultMessage):
                result = message

    usage = {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
    u = getattr(result, "usage", None) if result else None
    if isinstance(u, dict):
        usage["prompt_tokens"] = int(u.get("input_tokens", 0) or 0)
        usage["completion_tokens"] = int(u.get("output_tokens", 0) or 0)
        usage["total_tokens"] = usage["prompt_tokens"] + usage["completion_tokens"]
    if calls:
        # Handing the call to A3T interrupts Claude, which may mark the turn an error: it is not one.
        return text.strip(), calls, usage
    if error or (result is not None and getattr(result, "is_error", False)):
        # Claude failed (not logged in, rate limited…): an error, never an answer.
        raise RuntimeError(f"{error or 'error'}: {(getattr(result, 'result', None) or text or '').strip()}")
    final = (getattr(result, "result", None) or text or "").strip()
    return final, [], usage


# ── HTTP ─────────────────────────────────────────────────────────────────────

@app.get("/health")
def health():
    return {"ok": True, "scope": "a3t" if SCOPE_ON else "open", "local_tools": False}


@app.get("/v1/models")
def models():
    return {"object": "list", "data": [{"id": DEFAULT_MODEL, "object": "model", "owned_by": "anthropic"}]}


def completion(cid: str, created: int, model: str, text: str, calls: list[dict], usage: dict) -> dict:
    msg = {"role": "assistant", "content": text or None}
    if calls:
        msg["tool_calls"] = tool_calls_out(calls)
    return {
        "id": cid, "object": "chat.completion", "created": created, "model": model,
        "choices": [{"index": 0, "message": msg, "finish_reason": "tool_calls" if calls else "stop"}],
        "usage": usage,
    }


@app.post("/v1/chat/completions")
async def chat_completions(body: dict):
    messages = body.get("messages") or []
    model = body.get("model") or DEFAULT_MODEL
    system, prompt = render(messages)
    if not prompt:
        raise HTTPException(status_code=400, detail="no user, assistant or tool message to send")
    tools = tool_specs(body.get("tools"))

    try:
        text, calls, usage = await turn(system, prompt, model, tools)
    except Exception as exc:  # noqa: BLE001 — the caller sees what failed
        raise HTTPException(status_code=502, detail=f"claude error: {exc}") from exc

    cid, created = "chatcmpl-" + uuid.uuid4().hex[:24], int(time.time())
    out = completion(cid, created, model, text, calls, usage)
    if not body.get("stream"):
        return out

    def gen():
        delta = {"role": "assistant", "content": text or ""}
        if calls:
            delta["tool_calls"] = [{"index": i, **c} for i, c in enumerate(out["choices"][0]["message"]["tool_calls"])]
        for chunk in ({"delta": delta, "finish_reason": None}, {"delta": {}, "finish_reason": out["choices"][0]["finish_reason"]}):
            yield "data: " + json.dumps({"id": cid, "object": "chat.completion.chunk", "created": created, "model": model,
                                         "choices": [{"index": 0, **chunk}]}) + "\n\n"
        yield "data: [DONE]\n\n"

    return StreamingResponse(gen(), media_type="text/event-stream")
