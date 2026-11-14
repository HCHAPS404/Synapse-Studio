# Kiroween 2026 compliance matrix

Source: official rules (provided by the team on 2026-11-13). Re-check the rules page
for changes before submitting.

| Rule | How we comply | Verified by |
|---|---|---|
| Deadline 2026-12-04 14:00 PT (17:00 Colombia), no extensions | Version freeze 12-02, submit 12-03 | Plan gates G5 |
| New project, built in the submission period (from 2026-10-30) | Git history starts 2026-11-13; prior code disclosed in `NOTICE` | git log |
| Working app URL; free, unrestricted access until 2027-01-29 | Virtual lab without login; paid always-on backend; static web on CDN; offline package | Task 78, Task 76 |
| Hardware not widely available may require physical access on request | The product is fully functional without hardware (virtual devices); hardware is an extra shown in the video | Task 77, 78 |
| Public GitHub repo, open source license visible in About | Apache-2.0 `LICENSE` | CI `compliance` job |
| `/.kiro` at repo root, never gitignored | Hook + `tools/check-kiro-not-ignored.ps1` + CI | CI `compliance` job |
| Third-party SDKs/APIs used under their terms | Terms reviewed before use; product works without any AI provider | `docs/human-actions.md` |
| Original work, IP owned by team; open source licenses respected | DCO sign-off; SBOM and license report | CI (later) |
| Sufficient Kiro usage (otherwise disqualification) | Specs, steering, hooks, MCP, custom agents, PBT, powers, Crew, logged in `docs/kiro-log.md` | Task 75 write-up |
| English materials (video, description, testing instructions) | English-first UI and docs; Spanish as secondary locale | Steering `product.md` |
| Video under 3 minutes, YouTube public, no third-party trademarks or copyrighted music | Generic 3D models, covered logos, own/royalty-free music | Task 75 checklist |
| Student bonus: all members are students | LinkedIn, student email, year/degree, school for both | Devpost form |
| Social Blitz: post starts with `#KiroweenHackathon26`, own thoughts | Written by the team | Task 75 |
| Blog: mentions it was written for Kiroween 2026, published in the period | Written by the team | Task 75 |
| Kiro Crew app: separate repo, same GitHub account, new | Task 68 | Task 68 |
| No financial or preferential support from the sponsor | Do not use AWS commercial credits/funding for the project | Question 3 below |

## Questions for the administrator (send in writing)

1. Our project controls industrial machinery but is fully functional without it
   through built-in virtual devices and a digital twin. Is that sufficient with
   respect to the rule on hardware that is not widely available?
2. May we name device manufacturers descriptively (e.g. "compatible with a PLC from
   brand X") in the UI and in the video, with no logos?
3. Are the student credits and the hackathon Kiro credits compatible with the rule on
   financial or preferential support from the sponsor?
