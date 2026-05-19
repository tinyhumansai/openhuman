# Operations and Monitoring

This document describes the monitoring strategy, alert policies, and incident response procedures for the OpenHuman backend.

## Uptime Monitoring

OpenHuman uses external uptime monitors to ensure that critical backend services are available and performing within acceptable thresholds.

### Critical Endpoints

The following endpoints are monitored for uptime:

| Environment | Endpoint | Purpose | Threshold |
|-------------|----------|---------|-----------|
| **Production** | `https://api.tinyhumans.ai/health` | Public API liveness | < 5s response; 200 = healthy, 503 = alert |
| **Staging** | `https://staging-api.tinyhumans.ai/health` | Staging API liveness | < 10s response; 200 = healthy, 503 = alert |

### Monitoring Providers

1. **Pingdom (Primary)**:
   - Configured to hit the `/health` endpoints every 1 minute.
   - Alerts are triggered after 2 consecutive failures.
   - Monitors from multiple regions (US, EU, Asia).

2. **GitHub Actions (Secondary/Independent)**:
   - Scheduled workflow (`.github/workflows/uptime-monitor.yml`) runs every 5 minutes.
   - Serves as an independent signal from the deployment pipeline.

## Alerting and Escalation

### Alert Destinations

- **Slack/Discord**: Alerts are routed to the `#ops-alerts` channel.
- **Email**: Critical alerts are sent to `ops@tinyhumans.ai`.

### Escalation Path

1. **Level 1 (Immediate)**: Notification to `#ops-alerts`. On-call engineer acknowledges.
2. **Level 2 (15 minutes)**: Page to the lead backend engineer.
3. **Level 3 (30 minutes)**: Escalation to the CTO.

## Incident Response (Runbook)

When a monitor fires:

1. **Verify the outage**: Check the endpoint manually or via `curl -I <endpoint>`.
2. **Check Cloud Status**: Check [DigitalOcean Status](https://status.digitalocean.com/) or other upstream providers.
3. **Review Logs**: Access logs via the DigitalOcean dashboard or `ghcr.io` if applicable.
4. **Determine Scope**: Is it a total outage or degraded performance? Is it specific to a region?
5. **Mitigation**: Restart the service via the cloud console or redeploy the last known healthy tag.
6. **Communication**: Update the internal status and notify stakeholders if the outage exceeds 5 minutes.

## Testing Alerts

To test the alert pipeline safely without causing a real outage:
1. Temporarily change the monitor URL to a non-existent path (e.g., `/health-test-trigger`).
2. Verify that the alert is received in the designated channel.
3. Revert the change and verify the recovery notification.

## Maintenance

During planned maintenance, monitors should be paused to avoid false positives. This is handled via the provider's "Maintenance Mode" or by disabling the GitHub Action temporarily.
