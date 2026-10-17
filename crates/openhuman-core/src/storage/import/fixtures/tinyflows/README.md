# Legacy tinyflows stores

A workspace laid out as it was before tinyflows moved onto the storage ports,
written by the retired writers themselves (tinyflows v0.8.8: `tinyflows-sqlite`)
so the importer is tested against bytes the old code produced.

| Path | Holds |
| --- | --- |
| `workspace/flows/flows.db`, `workspace/flows/drafts/*.json` | two flows (one disabled and approval-gated), two revisions of `flow-alpha`, four runs, three state entries, two suggestions, two drafts |
| `workspace/flows/checkpoints.db` | a thread with a subgraph namespace, a thread whose id needs escaping, three pending writes |
| `workspace/cron/jobs.db` | a named shell job, an agent job with a channel origin, a flow-schedule job, four runs |
| `expected.json` | what the old readers returned for those files |

Nothing under here is written to: the tests copy the tree into a temp
directory, hash it before and after, and import from the copy.
