"""
README / Setup
1) Install Python 3.11+, FFmpeg with NVENC support, CUDA drivers (NVIDIA RTX), and Tesseract optional.
2) pip install -r requirements.txt
3) Fill config.yaml keys: OpenAI/Anthropic, ElevenLabs, Pexels/Pixabay, YouTube OAuth files.
4) Place branding assets under assets/branding and font under assets/fonts.
5) Run once: python main.py (it bootstraps DB and schedules jobs).
6) Keep this process alive (systemd/supervisor/docker). It runs autonomous loops.
"""

from __future__ import annotations

import asyncio
import logging
from pathlib import Path

import yaml
from apscheduler.schedulers.blocking import BlockingScheduler

from database.db import build_engine, build_session_factory, session_scope
from database.models import Base
from modules.idea_generator import IdeaGenerator
from modules.uploader import Uploader
from modules.video_assembler import VideoAssembler
from modules.voice_synthesizer import VoiceSynthesizer


def configure_logging(log_file: str):
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s %(levelname)s [%(name)s] %(message)s",
        handlers=[logging.FileHandler(log_file), logging.StreamHandler()],
    )


def load_config():
    with open("config.yaml", "r", encoding="utf-8") as f:
        return yaml.safe_load(f)


def run_pipeline(cfg: dict, sf):
    logger = logging.getLogger("pipeline")
    with session_scope(sf) as session:
        try:
            IdeaGenerator(cfg).generate_and_store(session)
        except Exception:
            logger.exception("Idea generation step failed")
    with session_scope(sf) as session:
        try:
            asyncio.run(VoiceSynthesizer(cfg).synthesize_pending(session))
        except Exception:
            logger.exception("Voice step failed")
    with session_scope(sf) as session:
        try:
            VideoAssembler(cfg).assemble_ready(session)
        except Exception:
            logger.exception("Video assembly step failed")
    with session_scope(sf) as session:
        try:
            Uploader(cfg).upload_due(session)
        except Exception:
            logger.exception("Upload step failed")


def main():
    Path("output").mkdir(exist_ok=True)
    cfg = load_config()
    configure_logging(cfg["app"]["log_file"])
    engine = build_engine(cfg["app"]["db_path"])
    Base.metadata.create_all(engine)
    sf = build_session_factory(engine)

    scheduler = BlockingScheduler(timezone=cfg["app"]["timezone"])
    scheduler.add_job(lambda: run_pipeline(cfg, sf), "interval", hours=6, id="full_pipeline", max_instances=1, coalesce=True)
    run_pipeline(cfg, sf)
    scheduler.start()


if __name__ == "__main__":
    main()
