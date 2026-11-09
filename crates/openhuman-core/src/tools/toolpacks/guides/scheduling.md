Reminders and scheduled jobs in OpenHuman's own scheduler. They run *later*; they cannot read the user's live calendar or meetings, which come from a connected calendar integration (`tool_search`).

1. **Ground the time.** Pass the user's phrase to `resolve_time` with their timezone (`current_time` gives it). Never hand-compute timestamps, and call it once per time.
2. **Confirm.** Propose the exact local time or cadence and get an explicit yes before creating anything.
3. **Create** with `cron_add`. `schedule` is a JSON **object**, never a string:
   - once: `{"kind":"at","at":"<rfc3339 from resolve_time>"}` (deleted after it runs)
   - repeating: `{"kind":"cron","expr":"<5-field cron>","tz":"<IANA zone>"}`, e.g. `"0 9 * * *"` daily 9 AM; or `{"kind":"every","every_ms":N}`. Agent jobs run at least 5 minutes apart.
   - an agent job sets `job_type:"agent"` and a `prompt` that tells the future run exactly what to deliver. Created in a conversation, it replies there on its own: leave `delivery` out, or set `{"mode":"proactive"}` for an in-app notification or `{"mode":"none"}` for silence.
4. **Change or cancel**: `cron_list` for the id, then `cron_update` (`job_id` and a `patch` of only what changes) or `cron_remove` (`job_id`). `cron_run` runs a job now; `cron_runs` shows its history.

Reminders and recurring messages always deliver. A conditional check puts its condition and "otherwise reply NO_REPLY" in its `prompt`; only such a job can skip a run. A finite repetition ("5 times") is a recurring job whose prompt says when to stop. Report the new job id and that it is listed under Settings → Cron Jobs. If a call fails, name the tool that failed and the next step.
