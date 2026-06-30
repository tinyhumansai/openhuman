#!/usr/bin/env python3
"""OpenHuman runtime Python server.

Private JSONL stdio protocol. Rust owns the process and sends one compact JSON
request per line. This server keeps expensive Python backends warm for the
life of the Rust core process.

Enabled backends are passed via the OPENHUMAN_RPS_BACKENDS env var (comma
separated, e.g. "spacy,kompress"). Each backend lazy- or eager-loads its model
as appropriate; spaCy loads at startup (fast), Kompress (torch/ModernBERT)
loads on first request so a missing/slow model never blocks the handshake.
"""

import json
import os
import re
import sys

PROTOCOL = 1
SPACY_MODEL = "en_core_web_sm"

_BACKENDS = [
    b.strip()
    for b in os.environ.get("OPENHUMAN_RPS_BACKENDS", "spacy").split(",")
    if b.strip()
]

_spacy_nlp = None

# Kompress (ModernBERT) lazy-loaded state.
_kompress_tok = None
_kompress_model = None
_KOMPRESS_MODEL = os.environ.get("OPENHUMAN_RPS_KOMPRESS_MODEL", "answerdotai/ModernBERT-base")
_KOMPRESS_DEVICE = os.environ.get("OPENHUMAN_RPS_KOMPRESS_DEVICE", "cpu")
_KOMPRESS_TARGET_RATIO = float(os.environ.get("OPENHUMAN_RPS_KOMPRESS_TARGET_RATIO", "0.5"))
_KOMPRESS_MAX_INPUT = int(os.environ.get("OPENHUMAN_RPS_KOMPRESS_MAX_INPUT_CHARS", "200000"))


def _emit(obj):
    sys.stdout.write(json.dumps(obj, ensure_ascii=False, separators=(",", ":")))
    sys.stdout.write("\n")
    sys.stdout.flush()


def _error(req_id, code, message):
    return {"id": req_id, "ok": False, "error": {"code": code, "message": str(message)}}


def _configure_stdio():
    if hasattr(sys.stdin, "reconfigure"):
        sys.stdin.reconfigure(encoding="utf-8")
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8")


# ── spaCy backend ────────────────────────────────────────────────────────


def _load_spacy():
    global _spacy_nlp
    if _spacy_nlp is not None:
        return _spacy_nlp

    import spacy

    try:
        _spacy_nlp = spacy.load(SPACY_MODEL, disable=["parser"])
    except Exception:
        _spacy_nlp = spacy.load(SPACY_MODEL)
    return _spacy_nlp


def _spacy_extract(params):
    text = (params or {}).get("text") or ""
    nlp = _load_spacy()
    doc = nlp(text)
    entities = [
        {
            "text": ent.text,
            "label": ent.label_,
            "start": int(ent.start_char),
            "end": int(ent.end_char),
        }
        for ent in doc.ents
    ]
    seen = set()
    nouns = []
    for tok in doc:
        if tok.pos_ in ("NOUN", "PROPN") and not tok.is_stop and tok.is_alpha:
            key = (tok.lemma_ or tok.text).lower().strip()
            if len(key) >= 2 and key not in seen:
                seen.add(key)
                nouns.append(key)
    return {"entities": entities, "nouns": nouns}


# ── Kompress backend (ModernBERT salience compressor) ────────────────────

_SENT_RE = re.compile(r"(?<=[.!?])\s+|\n+")


def _pick_device():
    if _KOMPRESS_DEVICE != "auto":
        return _KOMPRESS_DEVICE
    try:
        import torch

        if torch.cuda.is_available():
            return "cuda"
        if getattr(torch.backends, "mps", None) and torch.backends.mps.is_available():
            return "mps"
    except Exception:
        pass
    return "cpu"


def _load_kompress():
    global _kompress_tok, _kompress_model
    if _kompress_model is not None:
        return
    import torch  # noqa: F401
    from transformers import AutoModel, AutoTokenizer

    device = _pick_device()
    _kompress_tok = AutoTokenizer.from_pretrained(_KOMPRESS_MODEL)
    _kompress_model = AutoModel.from_pretrained(_KOMPRESS_MODEL).to(device)
    _kompress_model.eval()
    _kompress_model._oh_device = device


def _split_sentences(text):
    parts = [p.strip() for p in _SENT_RE.split(text)]
    return [p for p in parts if p]


def _kompress_compress(params):
    params = params or {}
    text = params.get("text") or ""
    target_ratio = float(params.get("target_ratio", _KOMPRESS_TARGET_RATIO))
    max_input = int(params.get("max_input_chars", _KOMPRESS_MAX_INPUT))
    if len(text) > max_input:
        text = text[:max_input]

    sentences = _split_sentences(text)
    if len(sentences) <= 3:
        return {"compressed_text": text, "input_chars": len(text), "output_chars": len(text)}

    _load_kompress()
    import torch

    device = getattr(_kompress_model, "_oh_device", "cpu")
    with torch.no_grad():
        enc = _kompress_tok(
            sentences,
            return_tensors="pt",
            padding=True,
            truncation=True,
            max_length=128,
        ).to(device)
        out = _kompress_model(**enc)
        mask = enc["attention_mask"].unsqueeze(-1).float()
        summed = (out.last_hidden_state * mask).sum(dim=1)
        counts = mask.sum(dim=1).clamp(min=1.0)
        emb = summed / counts
        doc_mean = emb.mean(dim=0, keepdim=True)
        salience = (emb - doc_mean).norm(dim=1)

    n_keep = max(3, int(round(len(sentences) * target_ratio)))
    if n_keep >= len(sentences):
        out_text = text
    else:
        order = salience.argsort(descending=True).tolist()
        keep_idx = sorted(order[:n_keep])
        out_text = " ".join(sentences[i] for i in keep_idx)

    return {
        "compressed_text": out_text,
        "input_chars": len(text),
        "output_chars": len(out_text),
    }


# ── Dispatch ─────────────────────────────────────────────────────────────


def _handle(req):
    req_id = req.get("id")
    method = req.get("method")
    params = req.get("params") or {}
    if method == "spacy.extract":
        return {"id": req_id, "ok": True, "result": _spacy_extract(params)}
    if method == "kompress.compress":
        return {"id": req_id, "ok": True, "result": _kompress_compress(params)}
    return _error(req_id, "unknown_method", f"unknown runtime_python_server method: {method}")


def main():
    _configure_stdio()
    # Eager-load only the cheap, fast backends at startup. Kompress (torch) is
    # lazy-loaded on first request so a heavy import never blocks the handshake.
    try:
        if "spacy" in _BACKENDS:
            _load_spacy()
    except Exception as exc:
        _emit({"ready": False, "error": f"{type(exc).__name__}: {exc}"})
        return 1

    _emit({"ready": True, "protocol": PROTOCOL, "backends": _BACKENDS})
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except Exception as exc:
            _emit(_error(None, "bad_json", exc))
            continue
        if not isinstance(req, dict):
            _emit(_error(None, "bad_request", "request must be a JSON object"))
            continue
        try:
            _emit(_handle(req))
        except Exception as exc:
            _emit(_error(req.get("id"), type(exc).__name__, exc))
    return 0


if __name__ == "__main__":
    sys.exit(main())
