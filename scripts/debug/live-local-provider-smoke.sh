#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT_DIR"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 1
  }
}

need_cmd curl
need_cmd node
need_cmd cargo

discover_json_field() {
  local url="$1"
  local js="$2"
  curl -fsS "$url" | node -e "$js"
}

LM_MODEL="${OPENHUMAN_LIVE_LMSTUDIO_MODEL:-$(
  discover_json_field \
    "http://127.0.0.1:1234/v1/models" \
    'let data="";process.stdin.on("data",c=>data+=c).on("end",()=>{const body=JSON.parse(data);const model=(body.data||[]).map(x=>x.id).find(id=>id && !/embed/i.test(id));if(!model){process.exit(2)}process.stdout.write(model)})'
)}"

OLLAMA_MODEL="${OPENHUMAN_LIVE_OLLAMA_MODEL:-$(
  discover_json_field \
    "http://127.0.0.1:11434/api/tags" \
    'let data="";process.stdin.on("data",c=>data+=c).on("end",()=>{const body=JSON.parse(data);const model=(body.models||[]).map(x=>x.name).find(name=>name && !/embed/i.test(name));if(!model){process.exit(2)}process.stdout.write(model)})'
)}"

export OPENHUMAN_LIVE_LMSTUDIO_MODEL="$LM_MODEL"
export OPENHUMAN_LIVE_OLLAMA_MODEL="$OLLAMA_MODEL"

echo "LM Studio model: $OPENHUMAN_LIVE_LMSTUDIO_MODEL"
echo "Ollama model: $OPENHUMAN_LIVE_OLLAMA_MODEL"

GGML_NATIVE=OFF cargo test --manifest-path Cargo.toml \
  live_lmstudio_provider_streams_thinking_and_text -- --ignored --nocapture

GGML_NATIVE=OFF cargo test --manifest-path Cargo.toml \
  live_ollama_provider_streams_text -- --ignored --nocapture
