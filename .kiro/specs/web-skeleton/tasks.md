# Web skeleton: tasks

- [x] **W1.** Create `apps/web` (config, health client, page, layout, ESLint, tsconfig).
  Covers W-R1 to W-R6.
- [x] **W2.** `pnpm install`, `pnpm lint`, `pnpm typecheck`, `pnpm build` pass; run the app
  against the live Core API and check the page shows `ok`.
- [x] **W3.** Note in `docs/martin.md` that `apps/web` holds a placeholder to replace in G2;
  entry in `docs/kiro-log.md`; commit and push to `develop`.
- [x] **W4 (Hell).** Done 22 nov: https://synapse-studio-eight.vercel.app. Create the Vercel project with Root Directory `apps/web` and
  `NEXT_PUBLIC_API_URL` after the skeleton is on `main`; then update
  `SYNAPSE_CORS_ORIGINS` in `render.yaml` with the real Vercel URL.

No tests are added: the acceptance criteria are build, lint, typecheck and a live check.

## Results of W1 to W3

Next.js 16.4, React 19.3, TypeScript 6 and ESLint 9. `pnpm add` first picked TypeScript 7
and ESLint 10: typescript-eslint refuses TS 7 and the React ESLint plugins declare ESLint
up to 9, so both are pinned one major lower. Typecheck caught a narrowing error in the
page (fixed with a type guard). `next build` marks `/` as dynamic, so the build never
calls the API. A production server built with `NEXT_PUBLIC_API_URL` set to the Render URL
rendered "Connected - status ok", version 0.0.1, environment production, database ok
(474 ms).

Note: Next.js inlines `NEXT_PUBLIC_*` values at build time, also in server code. Changing
the variable in Vercel needs a redeploy.

`.env.example` was not added: the agent permissions deny writes to `.env.*`. The variable
is documented in `apps/web/README.md`.
