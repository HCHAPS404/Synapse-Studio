# Conventions

- Code, comments, identifiers, commit messages, docs and UI strings: **English**.
  Spanish only as translation files (`es` locale).
- Conventional Commits with DCO sign-off (`git commit -s`).
- Work spec-first: update `.kiro/specs/<feature>/` before implementing. Reference
  the spec in the PR.
- After finishing a task, append an evidence entry to `docs/kiro-log.md` (what
  Kiro feature was used, what it produced, what changed).
- Do not add tests unless they belong to the task's acceptance criteria; when they
  do, include property-based tests for invariants.
- Never commit secrets. Never add `.kiro` to `.gitignore`.
- Error codes use the form `NE####` and each has an entry in `docs/errors/`.
- Do not copy vendor manuals, datasheets or CAD; derive register maps with
  attribution and use generic models.
- Ask the human for actions that cannot be automated: registrations, forms,
  hardware sessions, social posts, blog posts, video recording.
