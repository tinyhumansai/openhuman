from __future__ import annotations

import logging
import random
import subprocess
from pathlib import Path

import requests
from moviepy.editor import AudioFileClip, CompositeVideoClip, TextClip, VideoFileClip, concatenate_videoclips
from tenacity import retry, stop_after_attempt, wait_exponential

from database.models import Script, Video

LOGGER = logging.getLogger(__name__)


class VideoAssembler:
    def __init__(self, config: dict):
        self.cfg = config
        Path("output/footage").mkdir(parents=True, exist_ok=True)
        Path("output/videos").mkdir(parents=True, exist_ok=True)

    @retry(stop=stop_after_attempt(5), wait=wait_exponential(multiplier=1, min=1, max=20))
    def _fetch_footage(self, query: str, script_id: int) -> Path:
        headers = {"Authorization": self.cfg["video"]["pexels_api_key"]}
        r = requests.get("https://api.pexels.com/videos/search", params={"query": query, "per_page": 1}, headers=headers, timeout=30)
        r.raise_for_status()
        url = r.json()["videos"][0]["video_files"][0]["link"]
        out = Path("output/footage") / f"{script_id}.mp4"
        out.write_bytes(requests.get(url, timeout=60).content)
        return out

    def _subtitle_clip(self, text: str, duration: float):
        return TextClip(text.upper(), font=self.cfg["video"]["subtitle_font"], fontsize=self.cfg["video"]["subtitle_size"], color=self.cfg["video"]["subtitle_color"], stroke_color="black", stroke_width=3).set_position(("center", "bottom")).set_duration(duration)

    def assemble_ready(self, session):
        rows = session.query(Video, Script).join(Script, Video.script_id == Script.id).filter(Video.status == "voiced").limit(10).all()
        for video, script in rows:
            try:
                footage = self._fetch_footage(script.idea, script.id)
                vc = VideoFileClip(str(footage)).resize((self.cfg["video"]["width"], self.cfg["video"]["height"]))
                ac = AudioFileClip(video.audio_path)
                clip = vc.subclip(0, min(vc.duration, ac.duration + 1)).set_audio(ac)
                sub = self._subtitle_clip(f"{script.hook} {script.retention_hook}", clip.duration)
                final = CompositeVideoClip([clip, sub])
                out = Path("output/videos") / f"video_{script.id}.mp4"
                tmp = out.with_suffix(".tmp.mp4")
                final.write_videofile(str(tmp), fps=self.cfg["video"]["fps"], codec="libx264", audio_codec="aac", logger=None)
                subprocess.run(["ffmpeg", "-y", "-i", str(tmp), "-c:v", "h264_nvenc", "-preset", self.cfg["video"]["nvenc_preset"], "-c:a", "aac", str(out)], check=True)
                tmp.unlink(missing_ok=True)
                video.video_path = str(out)
                video.status = "edited"
                script.status = "edited"
                LOGGER.info("Rendered %s", out)
            except Exception as exc:
                LOGGER.exception("Video render failed script=%s error=%s", script.id, exc)
