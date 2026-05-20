# Operations and Monitoring

This document describes the monitoring strategy, alert policies, and incident response procedures for the OpenHuman backend.

## Uptime Monitoring

OpenHuman uses external uptime monitors to ensure that critical backend services are available and performing within acceptable thresholds.

### Critical Endpoints

The following endpoints are monitored for uptime:

| Environment | Endpoint | Purpose | Health Signal |
|-------------|----------|---------|---------------|
| **Production** | `https://api.tinyhumans.ai/health` | Public API liveness | HTTP 200 = healthy; HTTP 503 = one or more components in error state (alert) |
| **Staging** | `https://staging-api.tinyhumans.ai/health` | Staging API liveness | HTTP 200 = healthy; HTTP 503 = one or more components in error state (alert) |

### Monitoring Providers

1. **Pingdom (Planned — not yet configured)**:
   - Planned to hit the `/health` endpoints every 1 minute from multiple regions (US, EU, Asia).
   - Alerts to be triggered after 2 consecutive failures.
   - No Pingdom configuration currently exists in this repository; no alerts will be sent until it is set up.

2. **GitHub Actions (Active)**:
   - Scheduled workflow (`.github/workflows/uptime-monitor.yml`) runs every 5 minutes.
   - Serves as an independent signal from the deployment pipeline.
   - On outage detection, automatically creates a labeled GitHub Issue (`bug`, `critical`, `ops`) titled **"CRITICAL: Backend Outage Detected"** and closes it when services recover, providing a durable incident log in the repository.

## Alerting and Escalation

### Alert Destinations

- **Slack/Discord**: Alerts are sent to the configured webhook (e.g. `#ops-alerts`) when the `ALERT_WEBHOOK_URL` GitHub secret is set. Set this secret in the repository settings pointing to your Slack incoming webhook or Discord server webhook URL. Alerts are skipped silently if the secret is not configured.
- **Email** *(planned)*: Email alerting to `ops@tinyhumans.ai` is not yet wired into the automated workflow. Until an email integration is added, the `ALERT_WEBHOOK_URL` webhook is the only active notification channel.

### Escalation Path

1. **Level 1 (Immediate)**: Notification to `#ops-alerts`. On-call engineer acknowledges.
2. **Level 2 (15 minutes)**: Page to the lead backend engineer.
3. **Level 3 (30 minutes)**: Escalation to the CTO.

## Incident Response (Runbook)

When a monitor fires:

1. **Verify the outage**: Check the endpoint manually or via `curl -I <endpoint>`.
2. **Check Cloud Status**: Check [DigitalOcean Status](https://status.digitalocean.com/) or other upstream providers.
3. **Review Logs**: Access runtime logs via the DigitalOcean console or your container runtime (e.g. `docker logs <container>` or Kubernetes pod logs for containers sourced from `ghcr.io`).
4. **Determine Scope**: Is it a total outage or degraded performance? Is it specific to a region?
5. **Mitigation**: Restart the service via the cloud console or redeploy the last known healthy tag.
6. **Communication**: Update the internal status and notify stakeholders if the outage exceeds 5 minutes.

## Testing Alerts

To test the GitHub Actions alert pipeline safely without causing a real outage:
1. In `.github/workflows/uptime-monitor.yml`, temporarily change one endpoint URL to a non-existent path (e.g., `/health-test-trigger`) and trigger the workflow manually via `workflow_dispatch`.
2. Verify that a GitHub Issue is created and an alert is received in the `#ops-alerts` channel.
3. Revert the URL change and trigger the workflow again; verify the issue is closed and a recovery notification is sent.

To test the Pingdom monitor, use Pingdom's built-in test-alert feature from the dashboard rather than changing the monitored URL.

## Maintenance

During planned maintenance, monitors should be paused to avoid false positives. This is handled via the provider's "Maintenance Mode" or by disabling the GitHub Action temporarily.
