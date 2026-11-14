---
inclusion: fileMatch
fileMatchPattern: "{apps,packages}/**/*.{ts,tsx}"
---

# TypeScript and web style

- `strict` TypeScript, no `any` without a justified comment. ESLint and Prettier
  must pass.
- Types for IR, manifests and API come from generated JSON Schema / OpenAPI; never
  hand-write them.
- UI strings go through `next-intl` with English as the default locale.
- Accessibility: keyboard navigation, visible focus, color-blind-safe signal
  colors, ARIA labels on canvas controls.
- Heavy work (simulation, parsing) runs in Web Workers via the WASM packages.
- The virtual lab must run fully in the browser and work offline (PWA).
