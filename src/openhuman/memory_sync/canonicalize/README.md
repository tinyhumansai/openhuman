# canonicalize/

Source-specific adapters that normalise upstream payloads (chat batches, email threads, documents) into a single shape — `CanonicalisedSource { markdown, metadata }` — that the chunker downstream slices into bounded chunks.

Adapters do not interpret content semantically; they only normalise shape and capture provenance. Scoring / extraction / summarisation happen later in the pipeline.

## Files

- [`mod.rs`](mod.rs) — `CanonicalisedSource` struct, generic `CanonicaliseRequest<P>` envelope, and `normalize_source_ref` helper shared by all adapters.
- [`chat.rs`](chat.rs) — chat transcripts (Slack / Discord / Telegram / WhatsApp) → Markdown of `## <ts> — <author>\n<body>` blocks. Sorts messages and captures `time_range`. Produces empty-input `Ok(None)`.
- [`document.rs`](document.rs) — single documents (Notion page, Drive doc, meeting note, uploaded file) → trimmed body Markdown. `time_range` collapses to a single point at `modified_at`.
- [`email.rs`](email.rs) — email threads (Gmail + generic) → per-message `---\nFrom: …\nSubject: …\nDate: …\n\n<cleaned-body>` blocks. Bodies pass through `email_clean::clean_body` first.
- [`email_clean.rs`](email_clean.rs) — pure-string helpers: `clean_body` (strip reply chains + footer/legal boilerplate), `truncate_body`, `md_escape`, `extract_email`, `parse_message_date`. Used by both the email canonicaliser and the `gmail-fetch-emails` bin.

## Output contract

The canonicalised Markdown carries no leading `# Header` line — provider/title metadata lives in YAML front-matter written by `content_store/compose.rs`. The chunker relies on the `##` prefix followed by a space (chat) and `---\nFrom:` (email) boundaries to split at message granularity.
