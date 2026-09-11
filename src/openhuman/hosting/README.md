# hosting

Puts a workspace on the internet. This domain is the seam between OpenHuman and
[`tinyhosts`](../../../vendor/tinyhosts), the unified hosting API: TinyHosts owns
everything about a provider, and this module owns everything about OpenHuman.

## Responsibilities

- Resolve the configured hosting account (`Account::from_config`) from
  `[hosting]`, falling back to the provider's own environment variables.
- Decide which directory an agent may deploy (`resolve_in_workspace`).
- Expose the nine `hosting_*` agent tools and describe their results to a model.

Everything else — Vercel's endpoints, the upload-then-build deployment protocol,
how a marketplace database is provisioned and connected, the order a launch runs
in — belongs to the crate, where it is provider-independent and tested against a
mock of the provider's REST API. Nothing here knows the word `readyState`.

## Key files

| File | Role |
| --- | --- |
| `mod.rs` | `Account` (credential resolution + the shared `dyn Host`) and `resolve_in_workspace`. |
| `tools.rs` | The nine agent tools. |
| `test.rs` | Account resolution, workspace containment, and each tool's contract. |

## Agent tools

| Tool | Effect |
| --- | --- |
| `hosting_launch_site` | Deploys a workspace directory as a live site, optionally provisioning a database and wiring it in, setting environment variables, and attaching domains. External effect. |
| `hosting_deployment_status` | Whether a build has finished. Read-only. |
| `hosting_list_deployments` | A site's recent deployments, newest first, with status and target. Read-only. |
| `hosting_rollback` | Points production back at an earlier deployment that already built. External effect. |
| `hosting_list_sites` | The sites on the account. Read-only. |
| `hosting_set_env` | Sets environment variables on an existing site. External effect. |
| `hosting_add_domain` | Attaches a custom domain. External effect. |
| `hosting_domain_status` | Whether a site's domains are verified and serving. Read-only. |
| `hosting_analytics` | Traffic over the last N days. Read-only. |

### Why there is a rollback and no separate promote

`Host::promote` is both: a rollback *is* a promote of an older deployment, and
the crate models it once rather than twice. The tool is named for the reason an
agent reaches for it.

`hosting_rollback` reads the deployment before promoting it and refuses one that
did not finish building. `hosting_list_deployments` returns failed and
still-building deployments too — they are the history an agent is reading — so
the id it picks is not necessarily one that can serve traffic, and promoting a
failed build would take the site down during an attempt to bring it back up. It
does **not** check that the deployment belongs to the named site: a deployment is
looked up by id alone and the provider's response may omit the project name, so
that check would refuse legitimate rollbacks. The provider owns it.

## Gating

Two gates, and both matter:

- The `hosting` Cargo feature (default-OFF, product-ON). Off, the domain is not
  compiled at all.
- A credential that actually resolves. `Account::from_config` returns `Ok(None)`
  when `[hosting].enabled` is false or no key is found anywhere, and
  `tools::ops` then registers nothing. A tool that is present and cannot work is
  worse than one that is absent, because a model retries it.

A *misconfigured* section — an unknown provider slug, a blank configured key — is
an error rather than a silent skip, logged at `warn` by the registry.

## Two things this domain will not do

- **Read a secret.** A managed database's connection string is injected by the
  provider into the site's environment. This process learns the *names* of the
  variables and never their values, which is why `hosting_launch_site` reports
  `DATABASE_URL` rather than a URL.
- **Deploy something the user did not name.** `resolve_in_workspace` refuses an
  absolute path, a `..` escape, and a non-directory. It is the only place that
  decides what may leave the machine, and a deployment uploads every byte under
  the directory it is given.

## Events

None. No `bus.rs`, no `EventHandler`. Approval routing is via the
`Tool::external_effect` hook the agent harness reads.
