---
description: >-
  Shape OpenHuman into a role-specific assistant for clinical work: persona,
  tight data boundaries, and the limits you must keep in mind.
icon: user-doctor
---

# Create a doctor-specific assistant

**Goal:** tailor OpenHuman for a clinician's workflow: a persona that speaks the part, memory scoped to the right sources, and privacy settings appropriate for sensitive information.

{% hint style="danger" %}
**Read this first.** OpenHuman is a general-purpose assistant, **not** a medical device and **not** a source of medical advice. It can hallucinate. Nothing here makes it safe for diagnosis, treatment decisions, or handling protected health information under a specific regulatory regime (HIPAA, GDPR, etc.). You are responsible for compliance, for clinical judgment, and for what data you let it touch. Treat its output as draft text a qualified human must verify.
{% endhint %}

With that boundary set, here's how to shape it responsibly.

***

## Prerequisites

* OpenHuman set up. See [Create my personal AI assistant](personal-assistant.md).
* A clear decision about **what data this assistant may and may not see**. For anything sensitive, plan to keep inference [local](local-model.md).

## Privacy implications

* Decide early whether any real patient data will ever be involved. If regulatory rules apply to your data, the safest posture is: **local model on, minimal integrations, read-only or supervised autonomy.**
* OpenHuman keeps memory local and redacts secrets/PII on save, but **that is not a compliance guarantee**. It is a general privacy design. Do not treat it as certified for regulated health data.
* If you route reasoning to the [OpenHuman backend](../features/privacy-and-security.md) (the default), relevant snippets are sent to the model provider for each turn. For sensitive material, turn on a [local model](local-model.md) so that work stays on-device.

***

## Steps

### 1. Lock down the boundaries before adding data

In **Settings → Agents → Agent access**:

* Set autonomy to **Read-only** (pure Q&A/drafting) or **Supervised** (drafting plus approved actions). Avoid **Full** for clinical use.
* Keep **workspace-only** on so the agent can't wander your disk.
* Keep the [Approval Gate](../features/approval-gate.md) on. Nothing gets *acted on* (files written, actions taken) without your yes. Note it gates **actions**, not network transport: prompts and attachments can still be sent upstream for inference.

### 2. Turn on local inference for sensitive work

Follow [Use OpenHuman with a local model](local-model.md) and pick at least **"memory + reflection"** so embeddings and background summarization stay on-device. Confirm status reads `ready`.

### 3. Give it a clinical persona

The assistant's tone and behavior come from an editable prompt (`SOUL.md`), with mission/values in `IDENTITY.md`. To make it clinical:

* Set a display name and description in **Settings → Personality**.
* Edit the behavior prompt (via the **Brain** page, `/brain`) to describe the role you want. For example: "You assist a physician with documentation and literature summaries. You always flag uncertainty, cite sources, and never present output as a diagnosis or treatment recommendation. You remind the user to verify clinically."

Bake the caveats **into the persona** so they show up in every reply, not just your memory.

### 4. Connect only the sources that belong

Add only the integrations relevant to the workflow (e.g. a reference/notes source), and **not** anything carrying data you're not cleared to process. Every integration is a separate, revocable OAuth grant.

### 5. Verify behavior with safe, synthetic inputs

Test with **made-up** cases, never real patient data, until you're satisfied with tone, caution, and citations.

***

## Success checks

* [ ] Autonomy is **Read-only** or **Supervised**, workspace-only is on, approval gate is on.
* [ ] Local AI is `ready` if you're keeping inference on-device.
* [ ] The persona reliably adds uncertainty flags and "verify clinically" language in replies to synthetic prompts.
* [ ] Only intended sources are connected.
* [ ] The assistant declines to present output as a diagnosis when tested.

## Common failures

| Symptom | Cause | Fix |
| ------- | ----- | --- |
| It states things with false confidence | Persona doesn't enforce caution | Strengthen `SOUL.md` to require uncertainty flags and citations |
| Sensitive text went to the cloud | Inference is on the default route | Turn on a [local model](local-model.md) and confirm `ready` before using sensitive input |
| It tried to take an action on its own | Tier too permissive | Drop to **Read-only**; keep the approval gate on |
| It "remembered" something it shouldn't have | A source with disallowed data was connected | Revoke the integration; already-ingested chunks are local and can be cleared from the workspace |

## Recovery

* **Instant containment:** set autonomy to **Read-only**. Acting stops on the next turn.
* **Pull a source:** revoke any integration from Settings; future syncs stop immediately.
* **Reset the persona:** the behavior lives in an editable file; revert your edits to return to default tone.

## See also

* [Keep sensitive data private](privacy-sensitive-data.md): the controls this guide composes.
* [Use OpenHuman with a local model](local-model.md): keeping sensitive inference on-device.
* [Approval Gate](../features/approval-gate.md): how actions are gated.
