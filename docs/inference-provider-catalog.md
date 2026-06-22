# Inference Provider Catalog

This document tracks the built-in BYOK inference-provider presets exposed in
Settings > AI. OpenHuman still supports arbitrary OpenAI-compatible endpoints
through a custom provider entry; this table is the first-class chip catalog.

## Phase 1 Presets

| Slug                | Label             | Endpoint                                                  | Auth style | Status  |
| ------------------- | ----------------- | --------------------------------------------------------- | ---------- | ------- |
| `openai`            | OpenAI            | `https://api.openai.com/v1`                               | bearer     | Shipped |
| `anthropic`         | Anthropic         | `https://api.anthropic.com/v1`                            | anthropic  | Shipped |
| `openrouter`        | OpenRouter        | `https://openrouter.ai/api/v1`                            | bearer     | Shipped |
| `orcarouter`        | OrcaRouter        | `https://api.orcarouter.ai/v1`                            | bearer     | Shipped |
| `gmi`               | GMI               | `https://api.gmi-serving.com/v1`                          | bearer     | Shipped |
| `fireworks`         | Fireworks         | `https://api.fireworks.ai/inference/v1`                   | bearer     | Shipped |
| `moonshot`          | Kimi (Moonshot)   | `https://api.moonshot.ai/v1`                              | bearer     | Shipped |
| `groq`              | Groq              | `https://api.groq.com/openai/v1`                          | bearer     | Shipped |
| `mistral`           | Mistral           | `https://api.mistral.ai/v1`                               | bearer     | Shipped |
| `deepseek`          | DeepSeek          | `https://api.deepseek.com/v1`                             | bearer     | Shipped |
| `together`          | Together AI       | `https://api.together.xyz/v1`                             | bearer     | Shipped |
| `google`            | Google Gemini     | `https://generativelanguage.googleapis.com/v1beta/openai` | bearer     | Shipped |
| `cerebras`          | Cerebras          | `https://api.cerebras.ai/v1`                              | bearer     | Shipped |
| `xai`               | xAI               | `https://api.x.ai/v1`                                     | bearer     | Shipped |
| `huggingface`       | Hugging Face      | `https://router.huggingface.co/v1`                        | bearer     | Shipped |
| `nvidia`            | NVIDIA            | `https://integrate.api.nvidia.com/v1`                     | bearer     | Shipped |
| `zai`               | Z.AI              | `https://api.z.ai/api/paas/v4`                            | bearer     | Shipped |
| `minimax`           | MiniMax           | `https://api.minimax.io/v1`                               | bearer     | Shipped |
| `stepfun`           | StepFun           | `https://api.stepfun.ai/step_plan/v1`                     | bearer     | Shipped |
| `kilocode`          | Kilo Code         | `https://api.kilo.ai/api/gateway`                         | bearer     | Shipped |
| `deepinfra`         | DeepInfra         | `https://api.deepinfra.com/v1/openai`                     | bearer     | Shipped |
| `novita`            | Novita            | `https://api.novita.ai/v3/openai`                         | bearer     | Shipped |
| `venice`            | Venice            | `https://api.venice.ai/api/v1`                            | bearer     | Shipped |
| `vercel-ai-gateway` | Vercel AI Gateway | `https://ai-gateway.vercel.sh/v1`                         | bearer     | Shipped |
| `sumopod`           | SumoPod           | `https://ai.sumopod.com/v1`                               | bearer     | Shipped |
| `modelscope`        | ModelScope        | `https://api-inference.modelscope.cn/v1`                  | bearer     | Shipped |

API keys are stored through the auth-profile store under `provider:<slug>`;
they are not read from environment variables by the desktop Settings flow.

## Deferred Transports

The following provider families need transport or credential work beyond a
static OpenAI-compatible or Anthropic-compatible preset:

| Provider / family                                      | Needed work                                                            |
| ------------------------------------------------------ | ---------------------------------------------------------------------- |
| OpenAI Codex / ChatGPT subscription                    | Continue extending existing `openai_oauth` support.                    |
| xAI OAuth, MiniMax OAuth, Qwen OAuth, Nous device-code | Add provider-specific OAuth or import flows.                           |
| Google Gemini CLI, Claude CLI, Copilot ACP             | Add subprocess or external-process transports.                         |
| AWS Bedrock Converse                                   | Add native AWS SDK / Bedrock Converse transport.                       |
| Azure Foundry                                          | Model dual OpenAI / Anthropic API modes and Azure credential handling. |
| GitHub Copilot                                         | Add GitHub-token import and provider transport.                        |
