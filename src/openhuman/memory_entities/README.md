# memory_entities

Md-backed registry of people and other named things. Replacement for the
SQLite-backed `people/` module — the user's vault is the source of truth
so Obsidian, grep, and vector search all see the data without going
through a separate store.

## On disk

```text
<content_root>/entities/<kind>/<canonical_id>.md
```

Each file:

```markdown
---
id: person:alice
kind: person
display_name: Alice Cooper
aliases:
  - Ali
emails:
  - alice@example.com
handles:
  - kind: slack
    value: U12345
created_at: 2026-05-23T22:00:00Z
updated_at: 2026-05-23T22:00:00Z
---

Free-form notes the user can edit in Obsidian. Preserved across upserts.
```

`kind` matches `memory::score::extract::EntityKind` so canonical ids the
scorer emits round-trip through here unchanged.

## API

| Function | Purpose |
| --- | --- |
| `put_entity(config, Entity) -> Entity` | Upsert. Preserves user-edited notes body. |
| `get_entity(config, kind, canonical_id) -> Option<Entity>` | Read by id. |
| `list_entities(config, kind) -> Vec<Entity>` | Walk a kind directory. |
| `lookup_alias(config, kind, needle) -> Option<Entity>` | Find by alias / email / handle value / display name (case-insensitive). |

## Layout

| Path | Role |
| --- | --- |
| [`mod.rs`](mod.rs) | Module root + re-exports. |
| [`types.rs`](types.rs) | `Entity`, `EntityKind`, `EntityHandle`. |
| [`store.rs`](store.rs) | Disk-backed read/write, YAML compose/parse, atomic upsert that preserves notes body. Tests. |

## Migration from `people/`

| `people/` | `memory_entities/` |
| --- | --- |
| `Person { id, display_name, primary_email, primary_phone, handles, created_at, updated_at }` | `Entity { id, kind: Person, display_name, emails, handles, aliases, created_at, updated_at }` |
| `Handle::IMessage(s)` | `EntityHandle { kind: "imessage", value: s }` |
| `Handle::Email(s)` | added to `emails` |
| `Handle::DisplayName(s)` | added to `aliases` |
| `PeopleStore::insert_person / lookup / get / list` | `put_entity / lookup_alias / get_entity / list_entities` |

The SQLite-backed `people/` keeps running in parallel — this is a scaffold,
not a cut-over.

## Layer rules

- Borrows nothing from memory_store internals beyond the
  `<content_root>` path (resolved via `Config::memory_tree_content_root`).
- No SQLite. No async. No upward deps.
- Filenames are content-addressed slugs of the canonical id; the
  authoritative id lives in the file's YAML `id:` field, so the on-disk
  layout can change without breaking parsers.
