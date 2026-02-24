# Models

Hardware abstraction: model providers, routing policies, and complexity signals.

## Related docs

- `agents.md`: per-agent model override.
- `product-spec.md`: multi-model design principle.

## Providers

Linggen supports multiple model providers:

| Provider | Type | Config key |
|:---------|:-----|:-----------|
| Ollama | Local | `provider = "ollama"` |
| OpenAI-compatible | Cloud/Local | `provider = "openai"` |
| Claude API | Cloud | `provider = "openai"` (OpenAI-compatible endpoint) |
| AWS Bedrock | Cloud | `provider = "openai"` (via proxy) |

Models are configured in `linggen-agent.toml`:

```toml
[[models]]
id = "local-qwen"
provider = "ollama"
url = "http://127.0.0.1:11434"
model = "qwen3:32b"

[[models]]
id = "claude"
provider = "openai"
url = "https://api.anthropic.com"
model = "claude-sonnet-4-6"
api_key = "sk-..."
```

Optional `context_window` override for models where provider API doesn't report size.

## Routing policies

Named policies control which model handles each request.

**Built-in policies**:
- `local-first` — prefer local models, fall back to cloud.
- `cloud-first` — prefer cloud models, fall back to local.

**Custom policies**:

```toml
[routing]
default_policy = "balanced"

[[routing.policies]]
name = "balanced"
rules = [
  { model = "qwen3:32b", priority = 1, max_complexity = "medium" },
  { model = "claude-sonnet-4-6", priority = 2 },
  { model = "claude-opus-4-6", priority = 3, min_complexity = "high" },
]
```

## Complexity signal

Estimated from:
- Prompt length.
- Tool call depth.
- Skill metadata (`model` hint in frontmatter).

Skills can declare `model: cloud` or `model: local` to influence routing.

## Per-agent model

Agents can specify `model` in frontmatter to override the routing policy:

```yaml
---
name: coder
model: claude-sonnet-4-6
---
```

## Implementation

- `config.rs`: `ModelConfig`, `RoutingConfig`, routing policy definitions.
- `agent_manager/models.rs`: multi-provider dispatch, streaming, model selection.
- `ollama.rs`: Ollama API client.
