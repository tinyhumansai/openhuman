# Cloud deployment

OpenHuman is a desktop app, but its **Rust core** (`openhuman-core`) is a
headless JSON-RPC server that can be hosted in the cloud. Deploying the core
separately is useful for:

- Multi-device access — point several desktop clients at the same hosted core
- Internal testers without local Rust toolchains
- Long-running cron jobs / webhooks that should outlive a laptop session

This guide covers three deploy paths, easiest first:

1. [DigitalOcean App Platform: one-click](#1-digitalocean-app-platform-one-click)
2. [DigitalOcean App Platform: manual via doctl](#2-digitalocean-app-platform-manual-via-doctl)
3. [Any VPS via Docker Compose](#3-any-vps-via-docker-compose)

What gets deployed in every path: a single container running
`openhuman-core serve` on port `7788`, behind the provider's TLS. The desktop
app already knows how to talk to a remote core — set
`OPENHUMAN_CORE_RPC_URL=https://your-host/rpc` and `OPENHUMAN_CORE_TOKEN=...`
in `app/.env.local` and launch.

---

## What you need before you start

| Setting                    | Required | Notes                                                                 |
|----------------------------|----------|-----------------------------------------------------------------------|
| `OPENHUMAN_CORE_TOKEN`     | yes      | Bearer token clients send to `/rpc`. Generate with `openssl rand -hex 32`. **Anyone with this token can drive the core.** |
| `BACKEND_URL`              | yes      | Tinyhumans backend the core talks to (`https://api.tinyhumans.ai` for prod). |
| `OPENHUMAN_APP_ENV`        | no       | `production` or `staging`. Defaults to `staging`.                     |
| `OPENHUMAN_CORE_HOST`      | no       | Defaults to `0.0.0.0` in the container.                               |
| `OPENHUMAN_CORE_PORT`      | no       | Defaults to `7788`.                                                   |
| `RUST_LOG`                 | no       | `info` is fine; `debug` for triage.                                   |

Endpoints exposed by the running container:

- `GET /health` — public liveness probe. Used by every deploy path's healthcheck.
- `POST /rpc` — bearer-protected JSON-RPC entrypoint.
- `GET /events`, `GET /ws/dictation` — public streaming channels.

The `OPENHUMAN_WORKSPACE` directory (`/home/openhuman/.openhuman` inside the
container) holds the core's config, sqlite databases, and skill state. **Mount
it on a persistent volume** in every production deploy or you will lose data on
restart.

---

## 1. DigitalOcean App Platform: one-click

Click the button below to create a new App Platform application from this
repository's [`.do/app.yaml`](../.do/app.yaml):

[![Deploy to DO](https://www.deploytodo.com/do-btn-blue.svg)](https://cloud.digitalocean.com/apps/new?repo=https://github.com/tinyhumansai/openhuman/tree/main)

Then, in the App Platform UI, **before the first deploy completes**:

1. Open the **Settings → App-Level Environment Variables** tab.
2. Replace the placeholder `OPENHUMAN_CORE_TOKEN` value with a strong secret
   (`openssl rand -hex 32`). Mark it encrypted.
3. If you are deploying staging, change `OPENHUMAN_APP_ENV` to `staging` and
   `BACKEND_URL` to `https://staging-api.tinyhumans.ai`.
4. Hit **Save** — App Platform redeploys with the new secret.

App Platform handles TLS, restart-on-crash, log streaming, and rolling
redeploys on `git push` (set `deploy_on_push: true` in `.do/app.yaml` to
opt-in).

> **Persistence note:** App Platform Basic does not provide block storage. The
> core's workspace lives in the container's ephemeral filesystem and is lost
> on redeploy. For durable storage, attach a managed database or upgrade to a
> tier that supports volumes. See the [Compose path](#3-any-vps-via-docker-compose)
> for a self-host alternative with persistent volumes out of the box.

---

## 2. DigitalOcean App Platform: manual via doctl

If you'd rather not click through the UI:

```bash
# One-time: install doctl and authenticate.
doctl auth init

# Edit .do/app.yaml — set OPENHUMAN_CORE_TOKEN to a real value (or pass it in
# at create time via --spec with envsubst). Then:
doctl apps create --spec .do/app.yaml

# Watch the build:
doctl apps list
doctl apps logs <app-id> --type build --follow
```

Update an existing app after editing the spec:

```bash
doctl apps update <app-id> --spec .do/app.yaml
```

---

## 3. Any VPS via Docker Compose

Works on any host with Docker Engine ≥ 24 and the Compose plugin —
DigitalOcean Droplet, Hetzner, Linode, EC2, a home server.

```bash
# On the server:
git clone https://github.com/tinyhumansai/openhuman.git
cd openhuman

# Configure secrets:
cp .env.example .env
# Edit .env — at minimum:
#   BACKEND_URL=https://api.tinyhumans.ai
#   OPENHUMAN_CORE_TOKEN=<openssl rand -hex 32>
#   OPENHUMAN_APP_ENV=production

# Build and start:
docker compose up -d

# Verify:
docker compose ps
curl -fsS http://localhost:7788/health
```

The Compose file ([`docker-compose.yml`](../docker-compose.yml)) maps the core
on `:7788`, mounts a named volume `openhuman-workspace` for persistence, and
sets `restart: unless-stopped` so the core comes back after host reboots.

### Updating

```bash
git pull
docker compose build
docker compose up -d
```

### Logs

```bash
docker compose logs -f openhuman-core
```

### Putting it behind TLS

Use Caddy, nginx, or Traefik as a reverse proxy in front of `:7788`. A minimal
`Caddyfile`:

```caddy
core.example.com {
  reverse_proxy localhost:7788
}
```

---

## Pointing the desktop app at a hosted core

In the desktop app's environment file (`app/.env.local`):

```bash
# Use the hosted core instead of spawning a local sidecar.
OPENHUMAN_CORE_RUN_MODE=external
OPENHUMAN_CORE_RPC_URL=https://core.example.com/rpc
OPENHUMAN_CORE_TOKEN=<the same token you set on the server>
```

Restart the desktop app. The provider chain in `App.tsx` will route all RPC
calls to the remote core; nothing else changes.

---

## Smoke test

The repo ships [`.github/workflows/deploy-smoke.yml`](../.github/workflows/deploy-smoke.yml),
which runs on every PR that touches the deploy artifacts. It builds the
Docker image, boots it, and polls `/health` — so a regression in the cloud
deploy path fails CI before it lands on `main`.

To run the same check locally:

```bash
docker build -t openhuman-core:smoke .
docker run -d --name oh-smoke -p 7788:7788 \
  -e OPENHUMAN_CORE_TOKEN=smoke-test-token \
  openhuman-core:smoke
# Wait ~15s for the binary to come up, then:
curl -fsS http://localhost:7788/health
docker rm -f oh-smoke
```
