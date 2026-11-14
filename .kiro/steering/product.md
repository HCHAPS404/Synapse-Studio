# Product: SYNAPSE Studio and Neuron

SYNAPSE Studio is a software-defined industrial automation framework for the
Kiroween 2026 hackathon (category Frankenstein). **Neuron** is a new programming
language and canonical IR that:

- compiles directly to native code for microcontrollers (bare metal),
- imports and emits third-party languages (IEC 61131-3 ST/SCL/Ladder/FBD/SFC,
  SPEL+, C/C++, Python, Verilog/VHDL, assembly, URScript, RAPID, KRL, G-code),
- verifies safety, electrical, timing and logic properties,
- simulates everything in an own 3D digital twin whose virtual devices run the
  generated code,
- deploys to real hardware through the Edge Daemon.

Slogan: "Design once. Simulate in 3D. Compile to any machine."

## Non-negotiables

- The product must be fully usable **without any hardware** (virtual lab for
  judges, no login). Real hardware is an optional extra.
- AI is optional and never in the safety path. Three modes: classic (no AI),
  assistant (proposes, user accepts, verifier checks), bring-your-own model.
- Safety: this is a design aid, not a certified safety tool. Never claim IEC 61508
  or ISO 13849 compliance. A hardwired E-STOP is the authority.
- All material shown to judges is in **English** (UI default, README, docs,
  video, Devpost text). Spanish is a secondary locale.
- No third-party trademarks or copyrighted music in demo assets. Device 3D models
  are generic. Vendor names appear only descriptively.
- Hackathon rules: repo public with Apache-2.0, `/.kiro` at the root and never in
  `.gitignore`, new work only (created after 2026-10-30).

## Users

Students and beginners (visual blocks, natural language), control engineers
(GRAFCET, state machines, native code), integrators and SMEs (vendor-neutral),
and evaluators (one-click virtual lab).
