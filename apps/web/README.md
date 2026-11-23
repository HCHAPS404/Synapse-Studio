# apps/web

Placeholder skeleton: a Next.js page that reads `GET /health` of the Core API. It proves
the Vercel, Render and Neon chain. Martin's G2 (IDE shell) replaces it. Spec:
`.kiro/specs/web-skeleton/`.

```powershell
cd apps/web
pnpm install
pnpm dev                       # http://localhost:3000
pnpm lint; pnpm typecheck; pnpm build
```

## Public API URL

`NEXT_PUBLIC_API_URL` is the Core API base URL (`src/lib/config.ts`, `getApiConfig`). Unset in
development it defaults to `http://localhost:8000`; in production a missing or non-http(s) value
is an error, never a silent default. To point dev at the deployed API, create an untracked
`.env.local` with `NEXT_PUBLIC_API_URL=https://synapse-core-jqqk.onrender.com`.

Vercel: Root Directory `apps/web`, preset Next.js, variable `NEXT_PUBLIC_API_URL` set to the
production API URL above.

## CORS

The browser calls the API cross-origin, so the API must allow this app's origin. The Core API
reads the allowed origins from `SYNAPSE_CORS_ORIGINS` (a JSON array) and applies them with
FastAPI's `CORSMiddleware` (`services/core/src/synapse_core/main.py`). `render.yaml` sets it to
the Vercel production origins:

```
SYNAPSE_CORS_ORIGINS=["https://synapsest.vercel.app","https://synapse-studio-eight.vercel.app"]
```

If this app is served from a new origin (a different Vercel project or a preview domain), add
that origin to `SYNAPSE_CORS_ORIGINS` in the Render dashboard (and in `render.yaml`) and
redeploy; otherwise the browser blocks the response. `localhost:3000` is allowed by the default
settings for local development.

## Guest flow (what a judge does)

The product is usable with no account. `src/lib/guest.ts` (`startGuestSession`) performs the
first step; the full guest-driven virtual lab UI is Martin's G6. End to end the flow is:

1. `POST /v1/guest` → a guest **token** and a sandbox (the token goes in
   `Authorization: Bearer <token>` on every later call).
2. `POST /v1/projects` with a name → a project bound to the guest's sandbox (quota-limited).
3. `POST /v1/projects/{id}/versions` with `{ "files": { "name.neuron": "..." } }` → an
   immutable version.
4. `POST /v1/versions/{id}/builds` with `{ "kind": "check" }` (or `"build"`/`"emit"`) → a build
   is queued; poll `GET /v1/builds/{id}` until `status` is `succeeded`/`failed`/`error`.
5. Optionally open `WS /v1/relay` and send a `hello`; a guest receives a `welcome` with the
   `guest` role and may subscribe to a cell's telemetry.

`tools/live-smoke.py` runs exactly this path against the deployed service (E7.3). The typed
client in `packages/sdk-ts` (`SynapseClient.startGuestSession`) is the same flow with types
generated from the OpenAPI schema; the G6 UI should build on it.
