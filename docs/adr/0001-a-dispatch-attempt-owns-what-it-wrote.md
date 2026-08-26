<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# A dispatch attempt owns what it wrote

Two presses of Cut for the same Job can be dispatching at once, and both share the two pieces of
state that stand between an unconfirmed dispatch and material cut twice: the in-doubt id a retry
goes out under, and the mark that tells the window-close guard something was started. The id was
keyed to the Job; the mark was keyed to the cutter, coarser still. So an answer about one press
cleared what another press of the same Job — or of any other Job on that cutter — was still using
(#290). A clear made on the strength of one press's outcome is now attributed to that press. A
cutter's own free snapshot remains authoritative for every mark on that cutter.

## Considered options

**Serialise presses per Job** — a per-Job lock held across the dispatch — would have made the
question disappear rather than answered it, and is the smaller change. Rejected because it says
nothing about the cutter-wide mark: a refusal of Job A must not clear a mark left by Job B on the
same cutter, whether A and B were serial or concurrent. It also cannot classify a press that has
already returned without a usable answer. The next press after such a group is idle is the retry
that may resolve it; a sibling already dispatching alongside it is not. (Presses to one host already
serialize on that host's connection lock, so extra network blocking is not the objection.)

**Refcount instead of identity** — one integer per Job and per cutter, incremented before the
dispatch and decremented by whoever finishes — was rejected for the marks because a poll clears
them wholesale on the cutter's own authority ("it would take a Job right now"), and a decrement
arriving after that clear consumes a *later* press's count. That is #290 again by a third route.
Identity makes a stale clear a no-op instead. In-doubt ownership is grouped by DispatchId because
pruning at `ID_RETENTION` and eviction at the capacity cap can replace an id while an older press is
still in flight. An answer about the old group neither blocks nor clears the replacement.

## Consequences

A concurrent press that returns without a settling answer keeps its DispatchId unresolved even
after its guard drops; a sibling's answer cannot erase the retry id. The next press after that group
has no in-flight work is classified as its retry, and a settling answer from that retry resolves the
group so a later identical Cut can be intentional again (#121).

The two unconfirmed outcomes are different. If the host may have seen the id, a retry can be
reported as a duplicate rather than cut again — the safe direction, and the one #121 asked to be
visible rather than silent. If the host was provably never reached, it has never seen the retained
id; the retry sends that first-seen id and is cut normally. Nothing extends past `ID_RETENTION`,
which is still where an id stops being one a retry can use.
