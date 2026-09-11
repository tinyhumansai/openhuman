# runtime/pool

The client for pooled inline execution. The pool itself — warm interpreter
children, the job protocol, backpressure, idle reaping, recycle-after-N — lives
in the `tinyruntime` module, where one implementation serves every language.

What is here is two decisions this core still owns.

## 1. Whether a language pools at all

| Language | Default | Why |
| --- | --- | --- |
| `node` | **on** | Each job runs in its own `worker_thread`: a fresh module graph and fresh globals per job, so reuse is safe. |
| `python` | **off** | Jobs share one interpreter. CPython has no worker-thread equivalent and no safe way to kill a running thread, so reuse leaks `sys.modules`, `os.environ`, logging handlers, and threads across unrelated runs. Opt in with `[runtime_pool.python] enabled = true`. |

`[runtime_pool] enabled = false` is the master switch and reverts every caller to
its legacy per-call spawn, with no behavioural change. Pooling is an optimisation
seam, not a dependency.

## 2. What a failure means for the caller

This is the subtle part, and the reason `PoolRunError` has three variants rather
than being one error type. Each drives different caller behaviour:

| Variant | The job… | The caller must… |
| --- | --- | --- |
| `PreDispatch` | provably never reached a worker | fall back to a per-call spawn — safe, because nothing ran |
| `PostDispatch` | reached a worker and **may have executed** | **not** retry, or it risks running someone's code twice |
| `Saturated` | was shed because the pool was full | **not** spawn — that reintroduces exactly the resident memory the pool caps. Report busy, or retry later |

`classify` maps the module's failures onto these. The default is `PreDispatch`,
and the asymmetry is deliberate: mistakenly treating a job as un-run costs one
extra fallback spawn, while mistakenly treating a run job as un-run duplicates
its side effects. Only the two signals the module states explicitly — capacity
and post-dispatch — move a failure out of the default.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | `PoolRunError`, the shared `run_inline` dispatch, `classify`, and `all_stats`. |
| `node.rs` / `python.rs` | Per-language `enabled()` and `run_inline()`; they differ only in which language they name. |
| `types.rs` | `PoolExecOutcome` (with `queue_wait` kept apart from `elapsed`), `PoolLang`, `PoolSettings`. |

## Notes

- **`queue_wait` is reported separately from `elapsed` on purpose.** A host that
  cannot tell a slow job from a busy pool will tune the wrong knob.
- **`all_stats` returns empty rather than failing** when the module is not
  loaded. A status surface wants to render "nothing running", not an error.
- **A worker is still a child of this process.** A TinyBus module is a `cdylib`
  loaded in-process, so the resident cost and the process-tree shape the
  `library-profile skill-run` gate asserts on are unchanged by the move.
