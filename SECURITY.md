# Security Policy

## Reporting a vulnerability

Please do not open a public issue for security problems. Use GitHub's private
vulnerability reporting ("Security" tab, "Report a vulnerability") for this
repository. We aim to acknowledge reports within 72 hours.

## Scope and safety

SYNAPSE Studio can generate code that controls physical machinery. Treat
generated artifacts as untrusted until reviewed, simulated and verified.

- Packages deployed to devices are signed; the Edge Daemon refuses unsigned or
  tampered packages.
- Remote control uses TLS, mutual authentication between the daemon and the
  relay, role-based access control and an audit log.
- Never expose Modbus, S7 or other industrial protocols to the public internet.
- The software emergency stop is supplementary; a hardwired emergency stop is
  always the authority.

## Secrets

Never commit API keys or tokens. User-provided model API keys (BYOK) are stored
encrypted on the client and are never logged.
