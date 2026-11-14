# Actions that need a human (Hell / Martin)

Agents cannot do these. Tick them as they are completed.

## Now (before 2026-11-14)
- [x] Public GitHub repository https://github.com/HCHAPS404/Synapse-Studio with
      `main` and `develop` pushed; GitHub detects the license as Apache-2.0 (2026-11-14).
- [x] Repo-local git identity set (hchaps404); `Cargo.toml` and CODEOWNERS updated.
- [ ] Add Martin's GitHub username to `.github/CODEOWNERS` if he joins.
- [ ] Add a repository description and topics in GitHub "About" (judges see it first).
- [ ] Protect `main` and `develop` (require PR, require CI "Hackathon compliance",
      "Rust workspace" and "Core API (Python)", no force push).
- [ ] Enable GitHub private vulnerability reporting and secret scanning.
- [ ] Devpost: register the team, designate Hell as Representative.
- [ ] Send the three written questions to the hackathon manager (see
      `docs/compliance.md` section "Questions for the administrator").
- [ ] Sign the team IP / prize-sharing agreement (template: `docs/team-agreement.md`).

## Accounts and services
- [ ] Neon: create project `synapse` (dev and judging branches). Put the connection
      string only in Render/GitHub secrets.
- [ ] Render: create the `synapse-core` service from `render.yaml` (starter plan,
      budget it until 2027-01-29). Set `SYNAPSE_DATABASE_URL`.
- [ ] Vercel: create the web project when `apps/web` exists (Task 61).
- [ ] Review the terms of use of Claude, Meta and Modal APIs before using them in the
      public demo. Use one Claude account for development only.

## Hardware lab (before 2026-11-17, Task 50)
- [ ] Send the exact references (model numbers, firmware versions) of every device:
      PLCs, robot arm, FPGA board(s), MCU boards (ST, TI, Arduino, Raspberry Pi, ESP32).
- [ ] Confirm available vendor software and licenses (TIA Portal, MPE720, Epson RC+,
      Vivado, PlatformIO).
- [ ] Prepare isolated lab network, 24 V and 3.3 V supplies, optocouplers, level
      shifters, fuses, hardwired E-STOP, USB-serial and RS-485 adapters, ST-Link/JTAG,
      logic analyzer and oscilloscope.

## Submission (late)
- [ ] Record the demo video (under 3 minutes, English, no third-party trademarks or
      copyrighted music, logos on physical equipment covered).
- [ ] Write the blog post and the social post (`#KiroweenHackathon26` at the very
      start) yourselves; judges evaluate original thinking, not AI text.
- [ ] Fill in the student details for both members in the Devpost form.
