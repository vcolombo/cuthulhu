<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Issue labels

Every issue carries **one type** and, where it is known, **one provenance**. Type says what the
issue is; provenance says what surfaced it. Two axes, no more — a label nobody filters on is a
label nobody maintains.

## Type — exactly one, always

| Label | Use it when |
|---|---|
| `bug` | Behaviour is wrong today, or is latent and will be wrong once a related feature ships. |
| `enhancement` | Something the software cannot do yet. |
| `refactor` | Internal restructuring with no user-visible behaviour change. |
| `question` | A decision has to be made before any code makes sense. |
| `documentation` | The change is to docs only. |

`refactor` is the one most often mislabelled. If a user could notice the difference from the
outside, it is `bug` or `enhancement`, not `refactor`. A refactor that also fixes a bug is a
`bug` — label what a reader needs to know first.

`question` is not "we are unsure how to build this". It is "we do not yet know what the right
behaviour is". [#68](https://github.com/vcolombo/cuthulhu/issues/68) is the reference case: a
product decision about whether cuttability follows the path or the stroke has to land before
any implementation is meaningful.

## Provenance — one when the source is known, none when it is not

| Label | Use it when |
|---|---|
| `architecture-review` | Surfaced by a codebase architecture review — friction, shallow modules, duplicated implementations. |
| `parity-review` | Surfaced by a functional parity review against comparable cutting software. |
| `code-review` | Surfaced by an automated or human review of a pull request. |

Provenance is for filtering a whole sweep in or out at once. A backlog of forty parity items
should not drown five architecture findings, and vice versa. These are issue-search queries, and
each one is copyable as written — GitHub search has no comment syntax, so nothing may be appended
to them:

| Query | Returns |
|---|---|
| `label:architecture-review label:refactor` | The findings ready to implement. |
| `label:architecture-review` | Everything that review produced, decisions included. |
| `-label:parity-review` | Everything that was not part of the parity sweep. |

**Do not guess.** Apply a provenance label only when the issue body says where it came from, or
when you filed it yourself and know. Issues #11 and #13 deliberately carry no provenance label:
both came from SP4 manual GUI verification, which is none of the three above. An issue with no
provenance label is a correct state, not an incomplete one.

## Adding a provenance label

Worth it when a sweep produces **five or more** issues that someone will later want to filter as
a group. Below that, the issue body saying where it came from is enough — three issues do not
need a taxonomy.

When you do add one, apply it to the whole sweep in the same pass. A provenance label covering
half its own sweep is worse than none, because filtering on it silently omits the rest.

## When you file an issue

1. Add the type. Every issue gets one.
2. Add a provenance label if the issue belongs to a sweep that already has one.
3. State the source in the body regardless — `From Codex review of PR #2`, `Surfaced by an
   architecture review (candidate 04 of 8)`. The body survives label churn, and it is what
   lets a future reader reconstruct provenance the labels do not cover.
4. Cite evidence as `path:line`. Re-check the lines before filing if the branch has moved —
   references drift fast, and a stale one costs the next reader more than it saved you.

## When you pick an issue up

Check the referenced lines still exist before trusting them. Issues outlive the code they
describe; several of the architecture-review issues were filed against line numbers that had
already shifted under a merge by the time they were written.
