from __future__ import annotations

import json
import logging
from typing import Any

from anthropic import Anthropic
from openai import OpenAI
from tenacity import retry, stop_after_attempt, wait_exponential

from database.models import Script

LOGGER = logging.getLogger(__name__)


class IdeaGenerator:
    def __init__(self, config: dict[str, Any]):
        self.config = config
        self.ai_cfg = config["ai"]

    @retry(stop=stop_after_attempt(5), wait=wait_exponential(multiplier=1, min=2, max=30))
    def _llm_call(self, prompt: str) -> str:
        if self.ai_cfg["provider"] == "anthropic":
            client = Anthropic(api_key=self.ai_cfg["anthropic_api_key"])
            msg = client.messages.create(model="claude-3-7-sonnet-latest", max_tokens=4000, messages=[{"role": "user", "content": prompt}])
            return msg.content[0].text
        client = OpenAI(api_key=self.ai_cfg["openai_api_key"])
        resp = client.responses.create(model=self.ai_cfg["model"], input=prompt)
        return resp.output_text

    def generate_and_store(self, session):
        for niche in self.config["niches"]["active"]:
            template = self.config["niches"]["prompt_templates"][niche]
            prompt = (
                f"{template}\nGenerate {self.ai_cfg['ideas_per_batch']} short video scripts in JSON array. "
                "Each object keys: idea, hook, content, retention_hook. Keep content <= 25 seconds speech."
            )
            try:
                raw = self._llm_call(prompt)
                scripts = json.loads(raw[raw.find("[") : raw.rfind("]") + 1])
            except Exception as exc:
                LOGGER.exception("Failed generating scripts for niche=%s: %s", niche, exc)
                continue
            for row in scripts:
                session.add(
                    Script(
                        niche=niche,
                        idea=row["idea"],
                        hook=row["hook"],
                        content=row["content"],
                        retention_hook=row["retention_hook"],
                        status="pending",
                    )
                )
            LOGGER.info("Stored %s scripts for niche=%s", len(scripts), niche)
