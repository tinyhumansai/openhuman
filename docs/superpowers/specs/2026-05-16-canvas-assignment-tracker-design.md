# Canvas Assignment Tracker Design

**Date:** 2026-05-16
**Primary repo:** `openhuman`
**Goal:** Add a read-only Canvas LMS tracker that monitors assignments for exactly two current courses and turns them into a clear local task list with urgency and reminders.

## User Need

The user wants OpenHuman to help manage school assignments without taking any action inside Canvas. The app should notice new work, show what is still pending, suggest what to start first, and remind the user before deadlines.

The system must never submit, delete, edit, message, mark complete, or otherwise write to Canvas. It only reads visible course and assignment information through Canvas APIs and stores user-owned planning state locally.

## Course Scope

The Canvas tracker is scoped to these two current Canvas courses only:

1. `361100-Secrets of the Soil-Lec.001 | 801[3/68]`
2. `515101-Radiation in Everyday Life-Lec.002[3/68]`

Every Canvas request must be filtered through a local course allowlist. If Canvas returns any other course, including starred old courses, published old courses, active-looking courses, or past enrollments, the tracker ignores it.

The third current course that lives in LINE is out of scope for this design.

## Architecture

The Canvas tracker should be a Rust core domain under `src/openhuman/canvas_tracker/`. Rust owns all business logic, persistence, Canvas API calls, safety policy, sync behavior, urgency calculation, and controller responses. React/Tauri only renders the task list and gathers local settings.

The domain exposes JSON-RPC controllers so the app and agent can query the tracker without duplicating rules:

- `openhuman.canvas_tracker_get_settings`
- `openhuman.canvas_tracker_update_settings`
- `openhuman.canvas_tracker_sync_now`
- `openhuman.canvas_tracker_list_tasks`
- `openhuman.canvas_tracker_update_local_status`
- `openhuman.canvas_tracker_list_reminders`

The implementation should register the controllers in the existing core registry and add schema/handler parity tests.

## Canvas API Access

Canvas access is read-only. The HTTP client used by this domain must only allow `GET` requests to the configured Canvas host.

Default Canvas host:

```text
https://mango-cmu.instructure.com
```

Allowed Canvas endpoints:

- `GET /api/v1/courses` to resolve current course ids and validate that the allowlisted courses exist.
- `GET /api/v1/planner/items` with `context_codes[]=course_<id>` for incomplete or new planner items.
- `GET /api/v1/courses/:course_id/assignments` for assignment details such as name, description, due date, lock date, points, and submission types.
- `GET /api/v1/courses/:course_id/assignments/:assignment_id` when a single assignment needs a detail refresh.

No endpoint outside the allowlist is allowed in the first version. Any attempt to use `POST`, `PUT`, `PATCH`, or `DELETE` must fail before the HTTP request is sent.

## Credentials

The user supplies a Canvas access token locally. The token must not be sent to the chat, logged, written to Markdown, stored in Redux, or placed in localStorage.

Preferred storage is OS keychain through the existing desktop credential path. If keychain integration is not available for this domain during initial implementation, the fallback is an encrypted local config entry in the OpenHuman workspace with logs redacting the full value.

The UI should show only connection status, host, and selected courses. It must never display the token after save.

## Local Data Model

The tracker stores normalized local records so planning state survives syncs:

```text
canvas_tracker_settings
canvas_tracker_courses
canvas_tracker_assignments
canvas_tracker_task_state
canvas_tracker_sync_runs
canvas_tracker_reminder_state
```

Each task row should support:

- `course_name`
- `course_id`
- `assignment_id`
- `assignment_name`
- `due_at`
- `due_at_unclear`
- `instructions_summary`
- `submission_type`
- `canvas_workflow_state`
- `canvas_submission_state`
- `local_status`
- `urgency_level`
- `recommended_start_at`
- `reminders_needed`
- `source_url`
- `last_seen_at`

Valid local statuses:

- `not_started`
- `in_progress`
- `waiting`
- `submitted`
- `done`
- `unclear`

Canvas submission data may set a task to `submitted` when the API clearly reports a submission. Canvas must not be updated when the user changes local status.

## Sync Behavior

Manual sync is required for the first version. Background sync is a separate follow-up design after the read-only path is proven.

Manual sync flow:

1. Load settings and token.
2. Resolve the two allowlisted courses.
3. Fetch planner items and assignments only for those course ids.
4. Normalize assignments into local task records.
5. Preserve existing local status unless Canvas clearly reports a submitted state.
6. Record a sync run with counts, failures, and ignored courses.
7. Return a user-readable summary.

If Canvas is unavailable, the token is invalid, or a course cannot be resolved, the sync should fail closed and keep the previous task list.

## Assignment Extraction

For every assignment, the tracker extracts:

- course name
- assignment name
- due date and time
- instructions summary
- submission type if visible
- urgency level
- recommended start date
- reminders needed

If the due date is absent, ambiguous, malformed, hidden behind overrides that cannot be resolved, or otherwise unclear, set `due_at_unclear = true` and display `unclear`. Do not infer a deadline from text.

Instruction summaries should be generated from Canvas assignment descriptions by stripping HTML, removing boilerplate, and keeping the expected deliverable, constraints, links, and submission notes. A deterministic summary is required for the first version; LLM summarization is a separate follow-up design behind a user-visible setting.

## Urgency And Start Dates

Urgency is deterministic:

- `critical`: due within 24 hours and not done or submitted.
- `high`: due within 3 days and not done or submitted.
- `medium`: due within 7 days and not done or submitted.
- `low`: due later than 7 days.
- `unclear`: due date is unclear.

Recommended start date:

- Start immediately for `critical`.
- Start today for `high`.
- Start 2 days before due date for small assignments.
- Start 4 days before due date when the description suggests multi-step work, projects, presentations, group work, quizzes with preparation, or uploaded files.
- Use `unclear` when the due date is unclear.

## Reminders

The first version produces reminder recommendations; it does not need OS notifications yet.

Reminder recommendations:

- On new assignment discovery.
- 3 days before due date.
- 24 hours before due date.
- Morning of due date.
- Immediately when a task is `not_started` and due within 24 hours.
- Immediately when due date is unclear.

Future work may wire these recommendations into OpenHuman notifications or cron automation after the read-only tracker is stable.

## UI

Add a Canvas Tracker surface that shows:

- Connection status.
- The two allowlisted courses.
- Last sync time.
- Sync now button.
- Task table sorted by urgency and due date.
- Local status selector.
- Filters for `not_started`, `in_progress`, `submitted`, `done`, and `unclear`.

The task table columns:

- Course
- Assignment
- Due
- Summary
- Submission
- Status
- Urgency
- Start
- Reminders

The UI should make ignored courses visible in sync details only as counts or names, so the user can tell the allowlist is working without cluttering the task list.

## Safety Rules

The safety policy is code-level:

- Only the configured Canvas host is allowed.
- Only the four read-only endpoint families listed above are allowed.
- Only `GET` is allowed.
- Course ids must be resolved from the two-course allowlist before assignment fetches run.
- Unknown courses are ignored.
- Canvas token values are redacted in logs and errors.
- Local status changes never call Canvas.
- Sync failures keep the last known task list instead of deleting data.

## Testing

Rust unit tests should cover:

- Course allowlist matching.
- Rejection of non-GET requests.
- Rejection of disallowed hosts and endpoints.
- Due date parsing and `unclear` behavior.
- Urgency and recommended start date rules.
- Preservation of local task status across sync.
- Sync failure leaving previous tasks intact.

Rust integration tests should use a mock Canvas server. No test should call real Canvas.

Frontend tests should cover:

- Settings render without exposing token.
- Task table sorting.
- Local status update.
- Sync error display.

## Out Of Scope

- LINE course tracking.
- Canvas write actions.
- Submitting assignments.
- Editing Canvas planner items.
- Marking Canvas tasks complete.
- Reading grades.
- Automatic OS notifications in the first implementation.
- LLM-generated summaries in the first implementation.

## References

- OpenHuman repo architecture and `AGENTS.md` instructions.
- Canvas Courses API: `GET /api/v1/courses` returns active courses for the current user and supports enrollment filters.
- Canvas Planner API: `GET /api/v1/planner/items` supports `context_codes[]` and incomplete item filters.
- Canvas Assignments API: assignments expose due dates, descriptions, submission types, lock dates, and related metadata.
