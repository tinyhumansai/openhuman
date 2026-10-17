---
description: >-
  Put a workspace folder on the internet: deploy a site, set up a database,
  set environment variables, attach a domain, and roll back.
icon: cloud-arrow-up
---

# Hosting

The agent can take a folder in your workspace and make it a live site, with a database behind it if the app needs one. Ask for it in chat. The deploy is a tool call like any other, and an irreversible one goes through the [approval gate](approval-gate.md).

{% hint style="info" %}
Vercel is the only provider today. The design is provider-neutral, and Railway, Cloudflare and a self-hosted target are planned, but nothing else works yet.
{% endhint %}

## What the agent can do

| Tool | Effect |
| --- | --- |
| `hosting_launch_site` | Deploys a workspace directory as a live site. It can also set up a database and connect it, set environment variables, and attach domains. |
| `hosting_deployment_status` | Whether a build has finished. |
| `hosting_list_deployments` | A site's recent deployments, newest first, with status and target. |
| `hosting_deployment_logs` | A deployment's build and runtime log events. |
| `hosting_rollback` | Points production back at an earlier deployment that already built. |
| `hosting_list_sites` | The sites on the account. |
| `hosting_set_env` | Sets environment variables on an existing site. |
| `hosting_add_domain` | Attaches a custom domain. |
| `hosting_domain_status` | Whether a site's domains are verified and serving. |
| `hosting_analytics` | Traffic over the last N days. |

A launch runs five steps in the only order that works: create the site, create the database, connect the database before the build, set the environment, then build. A framework that reads its environment at build time breaks in any other order. The call returns while the build is still running, so the agent polls `hosting_deployment_status` and does not block.

There is a rollback but no separate promote, because a rollback is promoting an older deployment. It refuses a deployment that never finished building. Promoting a failed build would take the site down while you try to bring it back.

## What it will not do

- **Read a secret.** The provider puts a managed database's connection string into the site's environment. OpenHuman learns the variable names and never their values. That is why a launch reports `DATABASE_URL` and not a URL.
- **Deploy something you did not name.** The directory check refuses an absolute path, a `..` escape and anything that is not a directory. It is the one place that decides what may leave your machine, and a deployment uploads every byte under the folder it is given.

## Setting it up

```toml
[hosting]
provider = "vercel"
api_key = ""        # leave blank to read the provider's own env var
team = ""           # blank means your personal account
```

With `api_key` blank, the credential comes from `TINYHOSTS_VERCEL_TOKEN`, then `VERCEL_TOKEN`. A team account also reads `TINYHOSTS_VERCEL_TEAM_ID` or `VERCEL_TEAM_ID`.

`[components] hosting` is `true` by default and is the only on/off switch; a legacy `[hosting] enabled = false` is migrated once to `[components] hosting = false` (see [Components](../developing/components.md)). When it is off, or when no credential can be found, the ten tools are not registered at all. A tool that is present but cannot work is worse than a missing one, because the model keeps retrying it.

A misconfigured section is logged as an error and not skipped silently. That means an unknown provider name, or a key that is blank after trimming. Leaving `api_key = ""` is the supported way to defer to the provider's environment variable and is not a misconfiguration.

## See also

- [Cloud deploy](cloud-deploy.md): hosting the OpenHuman core itself, which is a different thing.
- [Approval gate](approval-gate.md): how a deploy gets your yes.
- [Coder](native-tools/coder.md): building the thing before you ship it.
