<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Triage Labels

The skills speak in terms of five canonical triage roles. This file maps those roles to the actual label strings used in this repo's issue tracker.

| Label in mattpocock/skills | Label in our tracker | Meaning                                  |
| -------------------------- | -------------------- | ---------------------------------------- |
| `needs-triage`             | `needs-triage`       | Maintainer needs to evaluate this issue  |
| `needs-info`               | `needs-info`         | Waiting on reporter for more information |
| `ready-for-agent`          | `ready-for-agent`    | Fully specified, ready for an AFK agent  |
| `ready-for-human`          | `ready-for-human`    | Requires human implementation            |
| `wontfix`                  | `wontfix`            | Will not be actioned                     |

When a skill mentions a role (e.g. "apply the AFK-ready triage label"), use the corresponding label string from this table.

These are workflow-state labels used only by the triage skills. They sit alongside — not in
place of — the type and provenance axes defined in `docs/issue-labels.md`; an issue in triage
still carries exactly one type label. This repo's `question` type is a product decision, not
"needs-info".

Edit the right-hand column to match whatever vocabulary you actually use.
