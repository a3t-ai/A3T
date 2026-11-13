# Claude Code as an OpenAI-compatible model for A3T (claude-agent-sdk).
# It is only the model: A3T's agent runtime calls the tools as the signed-in user.
# Claude Code's own tools and any other MCP servers are off (see wrapper/src/main.py).

FROM python:3.12-slim

RUN apt-get update && apt-get install -y \
    curl \
    git \
    && rm -rf /var/lib/apt/lists/*

# Node.js 22 (required by Claude Code CLI)
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install Claude Code CLI globally
RUN npm install -g @anthropic-ai/claude-code

# Poetry
RUN curl -sSL https://install.python-poetry.org | python3 -
ENV PATH="/root/.local/bin:${PATH}"

# Copy wrapper source
COPY wrapper/ /app
WORKDIR /app

# Install Python deps
RUN poetry install --no-root

# An empty working directory: no .mcp.json or CLAUDE.md is read (strict MCP, no setting sources).
RUN mkdir -p /workspace

ENV CLAUDE_CWD=/workspace
ENV PORT=8000
# Answer only about A3T (A3T_SCOPE=off for a general-purpose model).
ENV A3T_SCOPE=on

EXPOSE 8000

CMD ["poetry", "run", "uvicorn", "src.main:app", "--host", "0.0.0.0", "--port", "8000", "--workers", "1"]
