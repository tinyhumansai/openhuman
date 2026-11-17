---
description: >-
  Shape OpenHuman into an assistant for clinical work, with a persona, tight
  data boundaries and clear limits.
icon: user-doctor
---

# Create a doctor-specific assistant

This guide tailors OpenHuman to a clinician's workflow: a persona that speaks the part, memory limited to the right sources, and privacy settings suited to sensitive information.

{% hint style="danger" %}
OpenHuman is a general-purpose assistant. It is not a medical device and not a source of medical advice, and it can hallucinate. Nothing here makes it safe for diagnosis, treatment decisions, or handling protected health information under a regulation such as HIPAA or GDPR. You are responsible for compliance, clinical judgment and the data you let it touch. Treat its output as draft text that a qualified person must verify.
{% endhint %}

## Prerequisites

- OpenHuman set up. See [Create my personal AI assistant](personal-assistant.md).
- A clear decision about what data this assistant may and may not see. For anything sensitive, plan to keep inference [local](local-model.md).

## Privacy implications

- Decide early whether real patient data will ever be involved. If regulatory rules apply to your data, the safest setup is a local model, minimal integrations, and read-only or supervised autonomy.
- OpenHuman keeps memory local and redacts secrets and personal data on save, but that is not a compliance guarantee. It is a general privacy design, not a certification for regulated health data.
- By default, reasoning goes through the [OpenHuman backend](../features/privacy-and-security.md), which sends relevant snippets to the model provider on each turn. For sensitive material, turn on a [local model](local-model.md) so the work stays on your device.

## Steps

### 1. Lock down the boundaries before adding data

In **Settings → Agents → Agent access**:

- Turn the autonomy policy on and set its tier. In `config.toml`, set `[autonomy] enabled = true` with `level = "readonly"` (questions and drafting only) or `level = "supervised"` (drafting plus approved actions). Avoid `full` for clinical use. The policy is off until you set `enabled = true`.
- Keep `workspace_only` on so the agent cannot wander your disk.
- With the policy on, the [Approval Gate](../features/approval-gate.md) sits between the assistant and any acting call. At `readonly` the call is refused. At `supervised` the gate holds it for your yes, unless its tool is on the always-allow list. Clear that list in **Settings → Agent access** if you want every call reviewed. The gate covers actions, not network transport, so prompts and attachments can still be sent upstream for inference.

### 2. Turn on local inference for sensitive work

Follow [Use OpenHuman with a local model](local-model.md) and pick at least "memory + reflection", so embeddings and background summaries stay on your device. Confirm the status reads `ready`.

### 3. Give it a clinical persona

The assistant's tone and behavior come from an editable prompt (`SOUL.md`), with mission and values in `IDENTITY.md`. To make it clinical:

- Set a display name and description in **Settings → Personality**.
- Edit the behavior prompt on the **Brain** page (`/brain`) to describe the role. For example: "You assist a physician with documentation and literature summaries. You always flag uncertainty, cite sources, and never present output as a diagnosis or treatment recommendation. You remind the user to verify clinically."

Put the caveats in the persona so they appear in every reply, not only in your memory.

### 4. Connect only the sources that belong

Add only the integrations the workflow needs, such as a reference or notes source. Do not connect anything with data you are not cleared to process. Every integration is a separate OAuth grant that you can revoke.

### 5. Test with synthetic inputs

Test with made-up cases, never real patient data, until you are satisfied with the tone, caution and citations.

## Success checks

- [ ] `[autonomy] enabled = true`, `level` is `readonly` or `supervised`, and `workspace_only` is on.
- [ ] If you keep inference on-device, your local runtime is running, the models are pulled, and a turn routed to it answers.
- [ ] The persona reliably adds uncertainty flags and "verify clinically" language to replies on synthetic prompts.
- [ ] Only the intended sources are connected.
- [ ] The assistant declines to present output as a diagnosis when tested.

## Common failures

| Symptom | Cause | Fix |
| --- | --- | --- |
| It states things with false confidence | The persona does not enforce caution | Strengthen `SOUL.md` to require uncertainty flags and citations |
| Sensitive text went to the cloud | Inference is on the default route | Turn on a [local model](local-model.md) and confirm `ready` before using sensitive input |
| It tried to act on its own | The policy is off, or the tier is too permissive | Set `enabled = true` and `level = "readonly"` |
| It remembered something it should not have | A source with disallowed data was connected | Revoke the integration. Chunks already ingested are local and can be cleared from the workspace |

## Recovery

- **Stop acting at once.** Set `enabled = true` and `level = "readonly"` together. The level does nothing while the policy is off, so you need both. Acting stops on the next turn.
- **Pull a source.** Revoke any integration in Settings, and future syncs stop immediately.
- **Reset the persona.** The behavior lives in an editable file. Revert your edits to return to the default tone.

## See also

- [Keep sensitive data private](privacy-sensitive-data.md): the controls this guide combines.
- [Use OpenHuman with a local model](local-model.md): keeping sensitive inference on your device.
- [Approval Gate](../features/approval-gate.md): how actions are gated.
