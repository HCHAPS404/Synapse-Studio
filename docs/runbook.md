# Runbook — Core API in production (until 2027-01-29)

Operational guide for the judging window (2026-12-16 to 2027-01-29). The Core API
(`services/core`) runs on Render as the service `synapse-core`, backed by Neon Postgres
(branch `production`); the web app is on Vercel (`https://synapsest.vercel.app`). This is a
hackathon design aid, not a certified system; a hardwired E-STOP is always the authority (see
`docs/daemon.md`). See `docs/deploy.md` for environments and `render.yaml` for the Blueprint.

## URLs and dashboards

| Thing | Where |
|---|---|
| Core API | https://synapse-core-jqqk.onrender.com |
| API health | https://synapse-core-jqqk.onrender.com/health |
| Web | https://synapsest.vercel.app |
| Render service | Render dashboard → `synapse-core` |
| Neon project | `synapse-studio` (`muddy-glade-31845276`), AWS US East 2 (Ohio), branch `production` |
| CI | GitHub Actions on `HCHAPS404/Synapse-Studio` |

## What to watch

- **`GET /health`** — returns `{status, version, environment, database, sandbox}`. `status` is
  `ok` normally and `degraded` when `database.status == "error"`. The scheduled live smoke
  (`.github/workflows/live-smoke.yml`, E7.3) checks this plus a guest session, a guest build and
  the relay every 30 minutes and is the primary signal.
- **Database latency** — `health.database.latency_ms`. A persistent climb or `error` means Neon
  is suspended, over quota, or unreachable. (Known: Render is in Oregon, Neon in Ohio, so expect
  ~50–70 ms per round trip; see the region TODO in `render.yaml`.)
- **Render service health** — Render's own health check hits `/health`; a failing deploy is
  rolled forward/back in the Render dashboard (see below).
- **Neon usage** — storage and compute hours against the plan; a Free-plan compute suspended
  over quota answers SQLSTATE 53000 / HTTP 402. Spending caps are a human step (E7-H).
- **Rate-limit / quota errors** — `NE1103` (rate limited, `Retry-After`) and `NE1104` (guest
  quota) in logs are expected under load, not incidents by themselves.

## Logs

- Render: dashboard → `synapse-core` → Logs. The service logs structured JSON with a
  `request_id`; the logging filter redacts secrets and URL credentials (E2.1), so a connection
  string never appears. Never paste a raw `SYNAPSE_DATABASE_URL` anywhere.
- Correlate a client failure by its `X-Request-ID` response header with the `request_id` field
  in the logs.

## Roll back a Render deploy

Render keeps previous deploys. To roll back:

1. Render dashboard → `synapse-core` → **Events** (or **Deploys**).
2. Find the last known-good deploy and choose **Rollback to this deploy** (Render calls it
   "Redeploy"/"Rollback"). The old image starts again.
3. The pre-deploy command runs `alembic upgrade head` on each deploy. A rollback to an older
   image expects an older-or-equal schema; because every migration is proven reversible in CI
   (the `migrations` job, E7.1), if the newer deploy had migrated the database forward, first
   bring the schema back to what the old image expects:
   ```powershell
   # from services/core, SYNAPSE_DATABASE_URL pointing at production (use with care)
   uv run alembic downgrade <target_revision>
   ```
   Pick `<target_revision>` as the `down_revision` the old image shipped (the highest migration
   present in that older commit). Only downgrade when a rollback actually needs it; a no-op
   rollback (same schema) needs no downgrade.
4. Confirm `GET /health` is `ok` and run the smoke manually:
   `python tools/live-smoke.py https://synapse-core-jqqk.onrender.com`.

To stop auto-deploys while investigating, set `autoDeploy: false` in `render.yaml` (or pause the
service in the dashboard) and resume when fixed.

## Recover the database

Neon keeps point-in-time history (instant restore / branching). Options, least to most drastic:

1. **Read a past state**: create a branch from an earlier time and inspect it without touching
   `production`:
   ```bash
   neon branches create --project-id muddy-glade-31845276 --name recover --parent production \
     --timestamp 2027-01-10T12:00:00Z
   neon connection-string recover --project-id muddy-glade-31845276 --pooled
   ```
2. **Restore `production` to a past state**: use Neon's restore (instant restore) in the Neon
   console to roll the `production` branch back to a timestamp. This is destructive to data
   written after that time — confirm with Hell first and announce downtime.
3. **Rebuild the schema from scratch** (empty database): `uv run alembic upgrade head` from
   `services/core` against the target URL builds every table; user/guest data is not recovered
   this way.

The audit log (`audit_log`) is append-only at the application layer (E3.4); do not `UPDATE`/
`DELETE` it during recovery.

## Incidents — quick triage

| Symptom | First check | Likely cause / action |
|---|---|---|
| Smoke `FAIL` (exit 1) | the smoke log | a real regression; check the last deploy, roll back |
| Smoke `SKIP` (exit 2) | Render status | transient; Render waking or a network blip, re-run |
| `/health` `degraded` | `database.status` | Neon suspended/over quota/unreachable; wake or check quota |
| HTTP 402 / SQLSTATE 53000 | Neon usage | Free-plan compute over quota; this service is paid — check plan |
| 429 / `NE1103` everywhere | load | rate limit working; raise limits only if deliberate |
| Web shows "Problem" | `NEXT_PUBLIC_API_URL`, CORS | wrong API URL or origin missing from `SYNAPSE_CORS_ORIGINS` |

## Uptime monitor

A documented config stub lives at `infra/monitoring/uptime-monitor.json` (provider-neutral).
Creating the external monitoring account and wiring alert channels is a human step (E7-H); see
that file's companion `infra/monitoring/README.md`. In short: monitor `GET /health` every 1–5
minutes, treat a non-200 or `status != ok` for 3 consecutive checks as an alert, and keep the
in-repo scheduled smoke (E7.3) as the deeper functional check.

## Shutdown plan (after judging, by 2027-01-29)

Judging ends 2027-01-29. To stop spending once it is over, in this order:

1. **Disable the scheduled smoke**: remove or comment the `schedule:` block in
   `.github/workflows/live-smoke.yml` (keep `workflow_dispatch` for manual runs).
2. **Disable the external uptime monitor** (pause or delete the check) so it stops alerting.
3. **Render**: either downgrade `synapse-core` from Starter to Free (it will spin down when
   idle, acceptable post-judging) or suspend/delete the service. Set `autoDeploy: false` if the
   service is kept but should not redeploy.
4. **Neon**: delete any extra branches (`dev`, `recover`), and either keep the `production`
   branch on the Free plan or export and delete the project. Scale-to-zero already stops idle
   compute; deleting removes storage cost.
5. **Vercel**: the web project can stay (free) or be removed. If removed, no action is needed on
   the API beyond the above.
6. Remove any still-live API keys: revoke the Neon MCP key (`neon api-keys revoke 3566976`) if
   not already done, and rotate `SYNAPSE_JWT_SECRET`/`SYNAPSE_SIGNING_KEY` if the service is
   kept running for any other purpose.

Record the shutdown date and what was torn down in `docs/kiro-log.md`.

## Related

- `docs/deploy.md` — environments, secrets, migrations, pooled vs direct URL.
- `render.yaml` — the Render Blueprint (pre-deploy `alembic upgrade head`, secret env vars).
- `tools/live-smoke.py` + `.github/workflows/live-smoke.yml` — the functional monitor (E7.3).
- `infra/monitoring/` — the uptime-monitor config stub and its setup notes (E7-H).
