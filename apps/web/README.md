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

Configuration: `NEXT_PUBLIC_API_URL` is the Core API base URL. Unset in development it
defaults to `http://localhost:8000`; in production it is required. To point dev at the
deployed API, create an untracked `.env.local` with
`NEXT_PUBLIC_API_URL=https://synapse-core-jqqk.onrender.com`.

Vercel: Root Directory `apps/web`, preset Next.js, variable `NEXT_PUBLIC_API_URL`.
