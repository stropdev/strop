# How Strop decides and ships work

[Issues](https://github.com/stropdev/strop/issues) track a problem, the observable outcome and what is left. [RFCs](docs/rfcs/README.md) record decisions and their rationale. [Milestones](https://github.com/stropdev/strop/milestones) answer *when*, not *whether it shipped*. The maintainer decides scope, accepts or rejects designs, and cuts releases. Agents may report, investigate, draft and implement; merging a draft is not design approval.

## Intake and triage

Use the [bug or proposal forms](https://github.com/stropdev/strop/issues/new/choose). Report suspected vulnerabilities **privately** under [SECURITY.md](SECURITY.md). An obvious correction with no behaviour change can go straight to a PR. For other changes, find or open an issue first.

| Label | Meaning |
|---|---|
| `needs-triage` | Verify the evidence or settle the scope. Not an accepted change. |
| `confirmed` | A fixed witness demonstrates a regression against an existing contract. Ready for a scoped fix; not approval for new behaviour. |
| `accepted` / `declined` | Maintainer disposition. On decline, explain why and close as not planned. |
| `needs-owner` / `blocked` | Name the exact decision or dependency. Remove the label when resolved. |
| `needs-rfc` / `rfc` | Design needed / tracking a specific RFC. Link file and issue both ways. |
| `needs-proof` | A changed kernel decision needs extraction, a theorem about the Rust, and a calibration mutant. |
| `static-review` | Identified by reading source. The issue says separately whether the effect was exercised. |
| `historical-backfill` | Retrospective record; read its disposition and evidence, not the issue creation/closure date. |
| `priority/first` | First pickup in the candidate release, not automatic acceptance. |

Use a type (`bug`, `enhancement`, `documentation`) and, where helpful, an `area/*` label. Remove `needs-triage` on disposition. A release candidate can be scheduled for investigation without declaring the report confirmed.

## Scheduling and closure

The named release milestone contains the proposed cut; `Later` has no release commitment. Keep each open outcome in one place. Every release milestone should say what to pick up first, what depends on it and how to tell it is done. No date is implied. Before closing a milestone, move unfinished work to the agreed release or `Later`, with a reason. Never close a ticket merely because the milestone or the RFC closed.

Use `Refs #N` for partial work and `Closes #N` only when the agreed outcome lands on `main`. Record design amendments, implementation and supersession in the RFC; record user-visible delivery in [CHANGELOG.md](CHANGELOG.md). For releases use the [release rules](AGENTS.md#release-gates), not an issue count.
