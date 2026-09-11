# Discussions

How this repository's Discussions are organized, and how a thread moves through
them. Written for maintainers doing triage; contributors only need
[SUPPORT.md](../../SUPPORT.md).

## Why not the default six

GitHub gives every repository the same categories — Announcements, General,
Ideas, Polls, Q&A, Show and tell. They sort by what kind of post something is,
which is the one distinction that does not help here: someone whose app will not
launch on macOS and someone whose recall degraded at 40k memories are both
"Q&A", need entirely different evidence, and are answered by different people.

Categories here follow the runtime, because the runtime decides what evidence
the thread needs and who can answer it.

## Categories

| Category | Slug | Answerable | For |
| --- | --- | --- | --- |
| Announcements | `announcements` | no | Releases and anything that changes how an existing install behaves. |
| Brain & memory | `brain-and-memory` | yes | Recall, the workspace, the durable journal, memory at scale. |
| Harness & plugins | `harness-and-plugins` | yes | The plugin surface, MCP servers, tools, the harness itself. |
| Models & providers | `models-and-providers` | yes | Model behavior under this harness; hosted, self-hosted, and local providers. |
| Install & platforms | `install-and-platforms` | yes | macOS permissions and sandboxing, Linux, Windows, Docker, mobile, updates. |
| Deep research | `deep-research` | yes | Multi-step research runs and sub-agent behavior. |
| Q&A | `q-a` | yes | Everything else someone is stuck on. |
| Show & tell | `show-and-tell` | no | Plugins, workflows, and setups. The plugin index is built from this category. |
| General | `general` | no | The catch-all. Triage moves posts out of it. |

Six have a form under
[`.github/DISCUSSION_TEMPLATE/`](../../.github/DISCUSSION_TEMPLATE). **The file
name must equal the category slug** — a template whose slug matches no category
is silently ignored, which is the first thing to check if a form disappears.

## Labels

Discussion labels are the repository's issue labels; these are the ones triage
runs on.

| Label | Meaning | Who sets it |
| --- | --- | --- |
| `awaiting maintainer` | Has replies, none from a maintainer. The triage queue. | Triage |
| `needs logs` | Cannot be acted on without the launch or harness log. | Anyone |
| `needs repro` | Nobody else has reproduced it yet. | Anyone |
| `plugin, not core` | The right shape for this is a plugin. | Maintainer |
| `local-first` | Turns on whether something is allowed to leave the machine. | Anyone |
| `promoted` | An issue was opened from this thread; the issue is linked. | Maintainer |

## Triage

Work the queue, not the feed:

1. **Wrong category** — move it. A misfiled thread is answered by nobody.
2. **`awaiting maintainer`, oldest first.** A thread with community replies and
   no maintainer reply is the failure this queue exists to catch; the built-in
   "unanswered" filter cannot see it, because those replies count as answers.
3. **Answerable and answered** — mark the answer, so the thread becomes the
   documentation for the next person with the same platform.
4. **A real defect** — open the issue, link both ways, label the thread
   `promoted`, and leave it open until the fix ships.
5. **Should be a plugin** — say so, label it `plugin, not core`, and leave it
   open. When someone builds it, it gets linked from that thread and posted to
   Show & tell.

Threads are not closed for age. A stale answered thread is documentation.

## Contributions and pull requests

External pull requests are limited today; the paths that are always open are a
discussion here and a plugin of your own. When triage closes the door on a code
change, it should point at whichever of those two actually fits — a "no" with no
route attached is how contributors leave.

## Cross-posting with OpenCompany

OpenHuman is the runtime inside [OpenCompany](https://github.com/tinyhumansai/opencompany),
so the launcher, the workspace root, and the agent journal produce questions
that genuinely belong to both repositories.

Post where you hit it and link the counterpart rather than closing it as a
duplicate — the repositories have different maintainers, and the link keeps both
able to answer. When the fix lands in the other repository, say so here and mark
the thread answered.

## Changing this setup

Categories cannot be created from the API — there is no GraphQL mutation for
them, so they are made in **Settings → Discussions** by someone with admin on
the repository. Anything added there needs a row in the table above, and a form
under `.github/DISCUSSION_TEMPLATE/` if the category expects evidence.
