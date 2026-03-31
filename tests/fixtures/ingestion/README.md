# Ingestion Fixtures

These fixtures are plain-text source samples for memory ingestion tests.

They are intentionally written as raw strings rather than strongly typed JSON so
future ingestion tests can exercise the same path used for real imported text.

Current fixtures:

- `gmail_thread_example.txt`
  Gmail-like thread with headers, quoted replies, task ownership, dates, and
  durable user/project facts.

- `notion_page_example.txt`
  Notion-like project page with sections, bullet lists, decisions, owners,
  milestones, and operating notes.

Suggested test usage:

- Load fixture text as a string.
- Pass it through chunking and extraction.
- Assert that ingestion can recover:
  - entities such as people, tools, projects, and dates
  - relations such as ownership, dependencies, and responsibilities
  - durable memory facts such as preferences, deadlines, and decisions
