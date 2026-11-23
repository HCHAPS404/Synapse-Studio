# Deployment and environments (Core API)

How the Core API (`services/core`) is deployed and how its database is configured per
environment. This is a design aid for the Kiroween hackathon, not a certified system; the
hardwired E-STOP is always the authority (see `docs/daemon.md`).

## Environments at a glance

| Environment | `SYNAPSE_ENVIRONMENT` | Where it runs | Database | Secrets |
|---|---|---|---|---|
| Production | `production` | Render service `synapse-core` (from `main`) | Neon branch `production` | real, set in the Render dashboard |
| CI | unset (`development`) | GitHub Actions | throwaway Postgres 16 service container | none real (container password is not a secret) |
| Local dev | unset (`development`) | your machine | Neon `dev` branch, local Postgres, or none | a per-process dev key is derived when unset |

The service reads every setting from environment variables prefixed `SYNAPSE_` (see
`services/core/src/synapse_core/settings.py`). Nothing is hard-coded and no secret is in the
repository. `services/core/.env.local` holds real values for local use and is git-ignored;
never read it into output and never commit any `.env*` file.

### Secrets the service needs

Listed in `render.yaml` as `sync: false` (no values in the repo). Set each in the Render
dashboard for production, or in `services/core/.env.local` for local use:

- `SYNAPSE_DATABASE_URL` — Postgres (Neon) URL. See pooled vs direct below.
- `SYNAPSE_JWT_SECRET` — ≥ 32-byte HMAC key for guest tokens (E2.8) and user JWTs (E3).
  Required when `SYNAPSE_ENVIRONMENT=production`; a stable per-process key is derived
  otherwise, so local runs and CI work with no secret.
- `SYNAPSE_SIGNING_KEY` — base64 Ed25519 32-byte seed for signing deployment packages (E5.2).
- `SYNAPSE_GUEST_IP_SALT` — salt for the one-way hash of a guest's IP (E2.8). Optional; set a
  stable value in production so guest IP hashes stay consistent across restarts.

Non-secret settings (`SYNAPSE_ENVIRONMENT`, `SYNAPSE_CORS_ORIGINS`) are inline in `render.yaml`.

## Migrations on deploy

Render runs `preDeployCommand: uv run --no-dev alembic upgrade head` (in `render.yaml`) once
after the build and before the new instance serves traffic, using the service's env vars. A
failed migration blocks the deploy, so new code never starts against an old schema; on an
already-migrated database the command is a no-op.

Reversibility is checked in CI: the `migrations` job (`.github/workflows/ci.yml`) runs
`alembic upgrade head` → `downgrade base` → `upgrade head` against a throwaway Postgres 16
service container. Running it keeps a Render rollback safe — an older image can downgrade to the
schema it expects. Run the same round trip locally against any Postgres with:

```powershell
# from services/core, with SYNAPSE_DATABASE_URL pointing at a throwaway database
uv run alembic upgrade head
uv run alembic downgrade base
uv run alembic upgrade head
```

## Pooled vs direct URL

Neon offers two host names for the same database:

- **Pooled** (`...-pooler.<region>.aws.neon.tech`): a PgBouncer endpoint, good for many short
  web connections. This is what the running service uses (`SYNAPSE_DATABASE_URL` on Render).
- **Direct** (`...<region>.aws.neon.tech`, no `-pooler`): a direct Postgres connection.

For this service either works for migrations: Alembic opens a single connection and runs DDL in
one transaction (`migrations/env.py` uses a `NullPool`), which the pooled endpoint handles in
session mode. If a future migration needs a feature the pooler rejects (advanced session state,
some `LOCK`/`SET` combinations), run that migration over the **direct** URL instead by pointing
`SYNAPSE_DATABASE_URL` at the no-`pooler` host for the `alembic upgrade` step. The app itself
should keep the pooled URL.

The driver dialect is normalised in code (`db/session.py` `sqlalchemy_url` rewrites
`postgresql://` to `postgresql+psycopg`), so you set a plain `postgresql://` URL and both the
async app engine and the synchronous Alembic engine use psycopg 3.

## CI uses a Postgres container, not Neon

The `core` and `migrations` jobs in CI start a `postgres:16` **service container** and point
`SYNAPSE_TEST_DATABASE_URL` / `SYNAPSE_DATABASE_URL` at `localhost:5432`. CI never connects to
Neon: it needs no Neon secret, every run starts from an empty database, and a flaky network or a
suspended Neon compute cannot break the build. The container password in the workflow is a
throwaway (`ci-only-not-a-secret`), not a real secret.

## Neon `dev` branch (for CI parity and local work)

Neon branches are copy-on-write clones of the `production` branch. A `dev` branch gives local
work and manual checks a realistic schema and data without touching production, and it can be
reset from its parent whenever it drifts.

Because the service reads the whole connection from `SYNAPSE_DATABASE_URL`, pointing it at a dev
branch "just works" — no code or config change. Put the dev branch's pooled URL in
`services/core/.env.local`:

```
SYNAPSE_DATABASE_URL=postgresql://<user>:<password>@<endpoint>-pooler.<region>.aws.neon.tech/<db>?sslmode=require
```

> **Human / CLI step (Hell).** Create the branch once with the Neon CLI. The automated agent
> does not create it (the MCP key may be revoked after setup; see the handoff notes). From a
> machine logged in to the Neon CLI:
>
> ```bash
> # Create a dev branch off production in the synapse-studio project
> neon branches create --project-id muddy-glade-31845276 --name dev --parent production
>
> # Print its pooled connection string (copy into services/core/.env.local as SYNAPSE_DATABASE_URL)
> neon connection-string dev --project-id muddy-glade-31845276 --pooled
>
> # When the dev branch drifts, reset it from production (safe; dev only)
> neon branches reset dev --project-id muddy-glade-31845276 --parent
> ```
>
> CI does **not** use the Neon dev branch; it uses the Postgres service container (above). The
> dev branch is for local runs and manual pre-production checks only. Keep spending caps in mind
> (E7-H): a dev branch adds a small amount of storage and compute.

## Related

- `render.yaml` — the Render Blueprint (service, build/start/pre-deploy commands, env vars).
- `docs/runbook.md` — what to watch, how to roll back a deploy, how to recover the database, and
  the shutdown plan after judging (E7.5).
- Web app configuration (public API URL, CORS): see `apps/web/README.md` and E7.4.
