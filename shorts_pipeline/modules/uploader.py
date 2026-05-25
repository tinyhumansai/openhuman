from __future__ import annotations

import logging
from datetime import datetime, timedelta, timezone

from google_auth_oauthlib.flow import InstalledAppFlow
from google.oauth2.credentials import Credentials
from googleapiclient.discovery import build
from googleapiclient.http import MediaFileUpload
from tenacity import retry, stop_after_attempt, wait_exponential

from database.models import Script, Video
from modules.thumbnail_generator import generate_thumbnail

LOGGER = logging.getLogger(__name__)
SCOPES = ["https://www.googleapis.com/auth/youtube.upload", "https://www.googleapis.com/auth/yt-analytics.readonly"]


class Uploader:
    def __init__(self, config: dict):
        self.cfg = config
        self.youtube = self._build_client()

    def _build_client(self):
        token_file = self.cfg["upload"]["youtube_token_file"]
        try:
            creds = Credentials.from_authorized_user_file(token_file, SCOPES)
        except Exception:
            flow = InstalledAppFlow.from_client_secrets_file(self.cfg["upload"]["youtube_client_secrets_file"], SCOPES)
            creds = flow.run_local_server(port=0)
            with open(token_file, "w", encoding="utf-8") as f:
                f.write(creds.to_json())
        return build("youtube", "v3", credentials=creds)

    def _next_schedule(self, idx: int):
        now = datetime.now(timezone.utc)
        hour = self.cfg["upload"]["publish_hours_utc"][idx % len(self.cfg["upload"]["publish_hours_utc"])]
        dt = now.replace(hour=hour, minute=0, second=0, microsecond=0)
        if dt <= now:
            dt += timedelta(days=1)
        return dt

    @retry(stop=stop_after_attempt(5), wait=wait_exponential(multiplier=1, min=2, max=60))
    def _upload_one(self, video: Video, script: Script):
        media = MediaFileUpload(video.video_path, chunksize=-1, resumable=True)
        title = script.hook[:95]
        tags = [script.niche.replace("_", " "), "shorts", "viral", "curiosidades", "fatos"]
        desc = f"{script.content}\n\n#{script.niche} #shorts"
        thumb_path = f"output/thumbnails/thumb_{script.id}.png"
        generate_thumbnail(title, thumb_path, self.cfg["video"]["subtitle_font"], self.cfg["video"]["primary_color"])
        req = self.youtube.videos().insert(part="snippet,status", body={"snippet": {"title": title, "description": desc, "tags": tags, "defaultLanguage": self.cfg["upload"]["default_language"]}, "status": {"privacyStatus": self.cfg["upload"]["channel_default_privacy"], "publishAt": video.scheduled_at.isoformat()}}, media_body=media)
        resp = req.execute()
        self.youtube.thumbnails().set(videoId=resp["id"], media_body=MediaFileUpload(thumb_path)).execute()
        return resp["id"], title, desc, tags, thumb_path

    def upload_due(self, session):
        queue = session.query(Video, Script).join(Script, Video.script_id == Script.id).filter(Video.status == "edited").limit(self.cfg["upload"]["max_videos_per_day"]).all()
        for i, (video, script) in enumerate(queue):
            video.scheduled_at = video.scheduled_at or self._next_schedule(i)
            try:
                yid, title, desc, tags, thumb = self._upload_one(video, script)
                video.youtube_video_id = yid
                video.title = title
                video.description = desc
                video.tags = ",".join(tags)
                video.thumbnail_path = thumb
                video.uploaded_at = datetime.utcnow()
                video.status = "uploaded"
                script.status = "uploaded"
                LOGGER.info("Uploaded script=%s youtube_id=%s", script.id, yid)
            except Exception as exc:
                LOGGER.exception("Upload failed for script=%s err=%s", script.id, exc)
