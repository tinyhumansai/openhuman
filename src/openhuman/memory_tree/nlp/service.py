#!/usr/bin/env python3
"""spaCy NER stdio service for the OpenHuman memory retriever.

Long-lived line-oriented JSON protocol over stdin/stdout. The model is loaded
exactly once at startup, then the process answers extraction requests until its
stdin is closed (the Rust parent drops the child on shutdown via kill_on_drop).

Protocol
--------
On startup, after the model loads, emit exactly one line:
    {"ready": true, "model": "en_core_web_sm"}
or, on fatal load failure:
    {"ready": false, "error": "<message>"}   (then exit non-zero)

For each request line `{"id": <str>, "text": <str>}`, reply with one line:
    {"id": <str>,
     "entities": [{"text": str, "label": str, "start": int, "end": int}, ...],
     "nouns": [str, ...]}

Entities are spaCy named entities (doc.ents). `nouns` are deduplicated lower-
case lemmas of common/proper nouns — E2GraphRAG keys retrieval on both named
entities and salient nouns, so a query like "migration runbook" still yields
graph anchors even with no PERSON/ORG spans.

All output is a single compact JSON object per line, flushed immediately
(the parent runs us with `python -u`, but we flush defensively anyway).
"""

import json
import sys

MODEL_NAME = "en_core_web_sm"
# Disable the parser/lemmatizer pipes we do not need for speed; keep the
# tagger (POS, needed for noun selection) and ner. `lemma` falls back to the
# surface form when the lemmatizer is absent, which is fine for our keys.
_DISABLE = ["parser"]


def _emit(obj):
    sys.stdout.write(json.dumps(obj, ensure_ascii=False))
    sys.stdout.write("\n")
    sys.stdout.flush()


def _load():
    import spacy

    try:
        return spacy.load(MODEL_NAME, disable=_DISABLE)
    except Exception:
        # Fall back to loading with the full pipeline if the disable list is
        # incompatible with the installed model build.
        return spacy.load(MODEL_NAME)


def _extract(nlp, text):
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
    return entities, nouns


def main():
    try:
        nlp = _load()
    except Exception as exc:  # pragma: no cover - exercised only without spaCy
        _emit({"ready": False, "error": f"{type(exc).__name__}: {exc}"})
        return 1

    _emit({"ready": True, "model": MODEL_NAME})

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except Exception as exc:
            _emit({"id": None, "error": f"bad request json: {exc}"})
            continue
        req_id = req.get("id")
        text = req.get("text") or ""
        try:
            entities, nouns = _extract(nlp, text)
            _emit({"id": req_id, "entities": entities, "nouns": nouns})
        except Exception as exc:  # pragma: no cover - defensive
            _emit({"id": req_id, "error": f"{type(exc).__name__}: {exc}"})

    return 0


if __name__ == "__main__":
    sys.exit(main())
