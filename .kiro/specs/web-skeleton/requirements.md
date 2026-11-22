# Web skeleton: requirements

Goal: a minimal, valid Next.js app in `apps/web` that Vercel can build and that reads
`GET /health` of the Core API. It proves the deployment chain (Vercel to Render to Neon)
before the IDE shell (Martin's G2) exists. It is a placeholder, not the product UI.

## Requirements

- **W-R1.** `apps/web` builds with `pnpm build` and passes `pnpm lint` and `pnpm typecheck`
  (strict TypeScript).
- **W-R2.** The API base URL is configured only through `NEXT_PUBLIC_API_URL`. In
  development it defaults to `http://localhost:8000`. In production a missing or invalid
  value shows a configuration error instead of silently using the default.
- **W-R3.** The home page shows the result of `GET <API>/health`: status, version,
  environment and database status. It renders a clear state when the API is unreachable,
  times out, or answers with an unexpected body. It never crashes the build.
- **W-R4.** The health request is made on the server (no CORS dependency), is never
  cached, and has a timeout.
- **W-R5.** No secret is read or shown. Only public values use the `NEXT_PUBLIC_` prefix.
- **W-R6.** The skeleton is self-contained (own lockfile, no root workspace changes) so
  Martin can replace it with the IDE shell without untangling it.

## Out of scope

Tailwind, shadcn/ui, next-intl, Zustand, TanStack Query, Monaco, dockview, PWA, WASM.
They belong to G2 (`docs/martin.md`). Types for API responses should be generated from
the OpenAPI schema; the one hand-written shape here is temporary and validated at runtime.
