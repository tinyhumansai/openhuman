# Memory tree — jobs handlers

Per-`JobKind` handler implementations dispatched by `worker::run_once_with_semaphore`. Each handler parses its payload, performs side effects, and enqueues any follow-up work (typically inside the same SQLite transaction as its primary write so a crash doesn't lose downstream jobs).

## Public surface

- `pub async fn handle_job(config, job)` — `mod.rs` — branches on `job.kind` and invokes the matching handler.

## Handlers (private to the module)

- `handle_extract` — runs the scorer + LLM extractor over one chunk, packs the embedding, writes `mem_tree_score` + entity-index rows + chunk lifecycle in one tx, and enqueues the follow-up `append_buffer` and `topic_route` jobs. Also rewrites Obsidian-style `tags:` in the on-disk chunk markdown (best-effort, post-tx).
- `handle_append_buffer` — hydrates a `LeafRef` (chunk or summary), pushes into the target tree's L0 buffer, and enqueues a `seal` job if the buffer crosses its budget. Updates chunk lifecycle (`buffered`) for source-tree appends. All in one tx.
- `handle_seal` — seals exactly one buffer level via `bucket_seal::seal_one_level` (which atomically inserts the parent-cascade seal and summary-side `topic_route` for source trees). Topic-tree seals are sinks and do not enqueue further routing. Rewrites tags on the sealed summary's `.md` post-commit.
- `handle_topic_route` — for each canonical entity associated with the node, asks the topic curator whether to spawn a topic tree, and enqueues an `append_buffer` per matched topic tree.
- `handle_digest_daily` — invokes `tree_global::digest::end_of_day_digest` for the requested UTC date; idempotent via the digest's own `find_existing_daily` check.
- `handle_flush_stale` — walks `list_stale_buffers` and enqueues a forced `seal` per buffer over the configured `DEFAULT_FLUSH_AGE_SECS` cap.

## Files

- `mod.rs` — `handle_job` dispatch and all handler bodies.
