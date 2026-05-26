from __future__ import annotations

import asyncio
import logging
from pathlib import Path

import aiohttp
from TTS.api import TTS
from tenacity import retry, stop_after_attempt, wait_exponential

from database.models import Script, Video

LOGGER = logging.getLogger(__name__)


class VoiceSynthesizer:
    def __init__(self, config: dict):
        self.cfg = config
        self.out_dir = Path("output/audio")
        self.out_dir.mkdir(parents=True, exist_ok=True)
        self.sem = asyncio.Semaphore(config["voice"]["concurrency"])

    @retry(stop=stop_after_attempt(5), wait=wait_exponential(multiplier=1, min=1, max=20))
    async def _elevenlabs(self, text: str, dst: Path):
        headers = {"xi-api-key": self.cfg["voice"]["elevenlabs_api_key"], "Content-Type": "application/json"}
        payload = {"text": text, "model_id": self.cfg["voice"]["elevenlabs_model_id"]}
        url = f"https://api.elevenlabs.io/v1/text-to-speech/{self.cfg['voice']['elevenlabs_voice_id']}"
        async with aiohttp.ClientSession() as s:
            async with s.post(url, headers=headers, json=payload, timeout=120) as r:
                if r.status >= 400:
                    raise RuntimeError(f"ElevenLabs error: {r.status} {await r.text()}")
                dst.write_bytes(await r.read())

    def _local_tts(self, text: str, dst: Path):
        model = "tts_models/multilingual/multi-dataset/xtts_v2"
        tts = TTS(model_name=model, gpu=True)
        tts.tts_to_file(text=text, file_path=str(dst))

    async def _process_script(self, session, script: Script):
        text = f"{script.hook}. {script.content}. {script.retention_hook}."
        audio_path = self.out_dir / f"script_{script.id}.mp3"
        try:
            async with self.sem:
                await self._elevenlabs(text, audio_path)
        except Exception as exc:
            LOGGER.warning("ElevenLabs failed for script=%s. Falling back local TTS. error=%s", script.id, exc)
            self._local_tts(text, audio_path)
        script.status = "voiced"
        video = session.query(Video).filter_by(script_id=script.id).one_or_none() or Video(script_id=script.id)
        video.audio_path = str(audio_path)
        video.status = "voiced"
        session.add(video)

    async def synthesize_pending(self, session):
        scripts = session.query(Script).filter(Script.status == "pending").limit(30).all()
        await asyncio.gather(*[self._process_script(session, s) for s in scripts], return_exceptions=True)
