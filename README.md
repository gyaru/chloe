![chloe](https://gbf.wiki/images/a/aa/Npc_zoom_3040530000_01.png)


### chloe

A Discord bot powered by LLM providers with tool calling capabilities.

#### configuration

**Required:**
- `REDIS_URL` - Redis connection string
- `POSTGRES_URL` - PostgreSQL connection string  
- `DISCORD_TOKEN` - Discord bot token

**LLM Providers (at least one required):**
- `GROQ_API_KEY` - Groq API key for Groq provider
- `ZAI_API_KEY` - z.AI API key for z.AI provider
- `OPENROUTER_API_KEY` - OpenRouter API key for OpenRouter provider

**Optional:**
- `LLM_PROVIDER` - Force specific provider (`groq`, `zai`, or `openrouter`). If not set, auto-detects based on available API keys
- `LLM_MODEL` - Specific model to use with any provider. Each provider has its own default if not set
- `EXA_KEY` - Exa search API key for web search functionality

#### Consistent Configuration Pattern

The bot follows a consistent `{SERVICE}_{SETTING}` pattern:
- `LLM_PROVIDER` + `LLM_MODEL` - For language model configuration
- `IMAGE_GEN_PROVIDER` + `IMAGE_GEN_MODEL` - For image generation (coming soon)
- More services will follow this same pattern

#### LLM Provider Support

**Groq Provider:**
- Models: llama-3.3-70b-versatile, llama-3.1-8b-instant, moonshotai/kimi-k2-instruct-0905, and more
- Supports: Tool calling ✅, Images ❌
- Default: moonshotai/kimi-k2-instruct-0905 (override with `LLM_MODEL=model-name`)

**z.AI Provider:**
- Models: GLM-4.5, GLM-4.5-Air  
- Supports: Tool calling ✅, Images ❌
- Default: GLM-4.5
- API: OpenAI-compatible via https://api.z.ai/api/coding/paas/v4

**OpenRouter Provider:**
- Models: 400+ models including Claude, GPT-4, Gemini, Llama, and more
- Supports: Tool calling ✅, Images ✅ (model-dependent)
- Default: openai/gpt-4o-mini (override with `LLM_MODEL=anthropic/claude-3.5-sonnet`)
- API: Unified access to multiple AI providers

#### Provider-Aware Intelligence

The bot now features **provider-aware processing** that optimizes behavior based on the LLM provider:

**z.AI/GLM & OpenRouter Models:**
- ✨ **Superior tool calling** - Uses modern structured tool calls
- 🚀 **Minimal processing** - Less workaround code needed
- 🎯 **Better reliability** - Modern models designed for agent applications
- 🖼️ **Vision support** - Many models can process images directly
- 🌐 **Model variety** - OpenRouter provides access to 400+ models

**Groq/Kimi Models:**
- 🔧 **Extensive workarounds** - Complex text-parsing fallbacks for inconsistent tool calls
- 📝 **Content filtering** - Removes unwanted prefixes and malformed responses
- 🛠️ **Fallback mechanisms** - Multiple pattern matching for broken tool call formats

#### Usage

The bot will automatically select an available provider and optimize its behavior:

1. If `LLM_PROVIDER` is set, it will use that specific provider
2. If not set, it auto-detects based on available API keys (priority: OpenRouter > z.AI > Groq)
3. **Provider-specific optimizations are applied automatically**

Simply set your API key(s) and the bot will handle the rest with optimal processing for each provider.

#### Model Examples

**Cross-Provider Model Usage:**
```bash
# Use Claude on any provider that supports it
LLM_MODEL=anthropic/claude-3.5-sonnet

# Use GPT-4o mini (cost-effective, available on OpenRouter/Groq)  
LLM_MODEL=openai/gpt-4o-mini

# Use GLM models (z.AI native, also available on OpenRouter)
LLM_MODEL=GLM-4.5
LLM_MODEL=z-ai/glm-4.5v

# Provider-specific models
LLM_MODEL=moonshotai/kimi-k2-instruct-0905  # Groq
LLM_MODEL=google/gemini-2.5-flash           # OpenRouter
LLM_MODEL=x-ai/grok-code-fast-1            # OpenRouter
```

**Smart Provider + Model Combinations:**
```bash
# High-performance coding with OpenRouter
LLM_PROVIDER=openrouter
LLM_MODEL=anthropic/claude-3.5-sonnet

# Budget-friendly with good tool calling
LLM_PROVIDER=openrouter  
LLM_MODEL=openai/gpt-4o-mini

# Vision tasks with z.AI
LLM_PROVIDER=zai
LLM_MODEL=GLM-4.5V

# Fast inference with Groq (with workarounds)
LLM_PROVIDER=groq
LLM_MODEL=llama-3.3-70b-versatile
```

#### Future: Image Generation

Coming soon with the same consistent pattern:
```bash
# Image generation configuration (planned)
IMAGE_GEN_PROVIDER=openrouter
IMAGE_GEN_MODEL=black-forest-labs/flux-1-dev

# Combined example
LLM_PROVIDER=openrouter
LLM_MODEL=anthropic/claude-3.5-sonnet
IMAGE_GEN_PROVIDER=openrouter  
IMAGE_GEN_MODEL=black-forest-labs/flux-1-schnell
```