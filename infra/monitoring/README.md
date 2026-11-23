# infra/monitoring

Monitoring configuration for the Core API during the judging window (until 2027-01-29).

## Files

- `uptime-monitor.json` — a provider-neutral **stub** describing the intended external uptime
  monitor for `GET /health`. It is not applied automatically; it is documentation you translate
  into whatever monitoring provider you use. No secret belongs in it.

## Two layers of monitoring

1. **Shallow uptime probe (external, this folder).** Hits `GET /health` every 1–5 minutes from
   outside the cluster and alerts when the API is down or reports `degraded`. This is the layer
   that pages a human. It needs an external account.
2. **Deep functional check (in-repo, E7.3).** `tools/live-smoke.py` run by
   `.github/workflows/live-smoke.yml` every 30 minutes: a guest session, a guest build and the
   relay handshake. It catches regressions a liveness probe cannot. It needs no external account.

## Human / CLI step (Hell) — E7-H

The external monitor and its spending caps are a human action, tracked as E7-H:

1. Create an account with an uptime provider (UptimeRobot, Better Stack, Checkly, Pingdom,
   Grafana Cloud Synthetics, ... — the free tier of any is enough for one check).
2. Create one monitor from `uptime-monitor.json`: `GET https://synapse-core-jqqk.onrender.com/health`,
   interval 60 s, expect HTTP 200 **and** the body to contain `"status": "ok"` (so a `degraded`
   database state also alerts).
3. Add an alert channel (email, or a webhook to Slack/Discord) and set
   "alert after 3 consecutive failures".
4. Set spending caps / budget alerts on Render and Neon (E7-H) so the paid plan cannot run away.
5. After judging (2027-01-29), pause or delete the monitor — see the shutdown plan in
   `docs/runbook.md`.

## Related

- `docs/runbook.md` — what to watch, roll back a deploy, recover the database, shutdown plan.
- `docs/deploy.md` — environments and secrets.
