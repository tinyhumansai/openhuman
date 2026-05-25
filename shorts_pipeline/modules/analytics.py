from __future__ import annotations

import json
import logging
from datetime import datetime, timedelta

from googleapiclient.discovery import build

from database.models import Analytics, Script, Video

LOGGER = logging.getLogger(__name__)


class AnalyticsModule:
    def __init__(self, config: dict, creds):
        self.cfg = config
        self.api = build("youtubeAnalytics", "v2", credentials=creds)

    def run(self, session):
        cutoff = datetime.utcnow() - timedelta(hours=self.cfg["analytics"]["min_age_hours"])
        rows = session.query(Video, Script).join(Script, Video.script_id == Script.id).filter(Video.status == "uploaded", Video.uploaded_at < cutoff).all()
        scored = []
        for video, script in rows:
            try:
                # Simplified demo query; dimensions and metrics can be expanded.
                resp = self.api.reports().query(ids="channel==MINE", startDate=(video.uploaded_at.date()).isoformat(), endDate=datetime.utcnow().date().isoformat(), metrics="views,averageViewDuration,averageViewPercentage", filters=f"video=={video.youtube_video_id}").execute()
                vals = resp.get("rows", [[0, 0.0, 0.0]])[0]
                metric = Analytics(video_id=video.id, views=int(vals[0]), avg_view_duration=float(vals[1]), retention_rate=float(vals[2]), ctr=0.0)
                session.add(metric)
                video.status = "analyzed"
                script.status = "analyzed"
                scored.append({"script_id": script.id, "niche": script.niche, "hook": script.hook, "views": metric.views, "retention_rate": metric.retention_rate})
            except Exception as exc:
                LOGGER.exception("Analytics failed for video=%s err=%s", video.youtube_video_id, exc)
        scored.sort(key=lambda x: (x["views"], x["retention_rate"]), reverse=True)
        self._write_report(scored)

    def _write_report(self, scored: list[dict]):
        with open(self.cfg["analytics"]["report_output_json"], "w", encoding="utf-8") as f:
            json.dump({"generated_at": datetime.utcnow().isoformat(), "top_videos": scored[:20]}, f, ensure_ascii=False, indent=2)
        with open(self.cfg["analytics"]["report_output_txt"], "w", encoding="utf-8") as f:
            f.write("Weekly Shorts Performance Report\n\n")
            for i, row in enumerate(scored[:20], 1):
                f.write(f"{i}. Script {row['script_id']} | {row['niche']} | Views={row['views']} | Retention={row['retention_rate']:.2f}\n")
