# @synapse/sdk-ts

TypeScript types and a thin typed client for the SYNAPSE Core API.

`src/schema.ts` is **generated** from `spec/api/openapi.json`, which is itself exported from
the FastAPI app (the single source of truth). Do not edit it by hand. `src/index.ts` is a
small hand-written fetch wrapper that re-exports those types.

## Regenerate

From this directory:

```bash
pnpm gen          # rewrite spec/api/openapi.json and src/schema.ts
pnpm gen:check    # verify both are up to date (what CI runs)
```

or from the repository root:

```bash
uv run --project services/core python tools/export-openapi.py
uv run --project services/core python tools/export-openapi.py --check
```

CI runs `--check` and fails on any `git diff`, so a Core API model change that is not
regenerated here breaks the build (requirement E-R5). The generator is pure Python and makes
no network call.
