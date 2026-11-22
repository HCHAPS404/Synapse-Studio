# Web skeleton: design

## Layout

```
apps/web/
  package.json, pnpm-lock.yaml, tsconfig.json, next.config.ts, eslint.config.mjs
  .env.example, README.md
  src/app/layout.tsx, src/app/page.tsx
  src/lib/config.ts     reads and validates NEXT_PUBLIC_API_URL
  src/lib/health.ts     fetchHealth(): never throws, returns a discriminated union
```

## Decisions

1. **App Router, server component.** `page.tsx` is `force-dynamic` and calls
   `fetchHealth()` on the server. No CORS, no client bundle for this call, no build-time
   fetch, so a sleeping or down API cannot fail the build.
2. **Result type.** `fetchHealth()` returns `{ kind: "ok" | "degraded", data }` or
   `{ kind: "unreachable" | "invalid" | "misconfigured", message }`. The page maps each
   kind to a message.
3. **Runtime validation** of the response with a small hand-written guard (no new
   dependency). It is temporary; generated OpenAPI types replace it later.
4. **Timeout** with `AbortSignal.timeout(8000)`. Render Starter is always on, but Neon may
   wake from idle on the first call.
5. **Config.** `NEXT_PUBLIC_API_URL`; default `http://localhost:8000` only when
   `NODE_ENV !== "production"`. The value must parse as an `http(s)` URL.
6. **Dependencies:** `next`, `react`, `react-dom`; dev: `typescript`, `@types/node`,
   `@types/react`, `@types/react-dom`, `eslint`, `eslint-config-next`. Prettier is left to
   Martin's G2 tooling.
7. **Vercel:** Root Directory `apps/web`, preset Next.js, Production Branch `main`,
   variable `NEXT_PUBLIC_API_URL`.
8. **CORS:** not needed by this page (server-side call). `SYNAPSE_CORS_ORIGINS` in
   `render.yaml` matters once browser code calls the API.
