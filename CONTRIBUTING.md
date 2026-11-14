# Contributing

Thank you for helping build SYNAPSE Studio and Neuron.

## Workflow

- Work spec-first with Kiro: a spec in `.kiro/specs/<feature>/` (requirements,
  design, tasks) precedes implementation. Log notable Kiro usage in
  `docs/kiro-log.md`.
- Branches: `main` (protected, deployed) and `develop` (integration). Short-lived
  branches branch from `develop`:
  `feat/<area>-<topic>`, `spec/<name>`, `fix/<topic>`, `hw/<device>-<date>`,
  `docs/<topic>`, `chore/<topic>`, `exp/<topic>`.
  Areas: `lang`, `sema`, `mir`, `backend-<arch>`, `std-<module>`, `engine`,
  `vdev`, `web`, `desktop`, `mobile`, `daemon`, `ai`, `infra`.
- Release branch: `release/kiroween-1.0`. Urgent fixes: `hotfix/<topic>`.
- `main` only receives merges from `develop` or `release/*`.
- Pull requests are required. Squash-merge into `develop`.

## Commits

- Conventional Commits (`feat:`, `fix:`, `docs:`, `chore:`, `test:`, `refactor:`).
- Sign off every commit (Developer Certificate of Origin):

  ```
  git commit -s -m "feat(lang): add lexer"
  ```

  By signing off you certify the contribution is yours to submit under the
  Apache License 2.0 (https://developercertificate.org/).

## Definition of done

1. Approved spec linked.
2. Unit and property-based tests pass; golden and conformance files updated.
3. Documentation updated; error messages carry `NE####` codes where applicable.
4. Review done (peer or Kiro agent) and CI green. No secrets and no third-party
   trademarks in assets. User-facing text in English (Spanish as translation).
5. Evidence recorded in `docs/kiro-log.md`.

## Hard rules

- Never add `.kiro` to `.gitignore`.
- Never commit secrets (API keys, tokens, `.env` files).
- Do not copy vendor documentation or CAD models that we cannot redistribute.
