<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# A dispatch attempt owns what it wrote

Two presses of Cut for the same Job can be dispatching at once, and both share the two pieces of
state that stand between an unconfirmed dispatch and material cut twice: the in-doubt id a retry
goes out under, and the mark that tells the window-close guard something was started. The id was
keyed to the Job; the mark was keyed to the cutter, coarser still. So an answer about one press
cleared what another press of the same Job — or of any other Job on that cutter — was still using
(#290). We gave a press an identity, and made every clear an attributed one: a press retracts the
mark carrying its own id and no other, and the Job's in-doubt entry is cleared by whichever
settling answer finds that every press still in flight has had one.

## Considered options

**Serialise presses per Job** — a per-Job lock held across the dispatch — would have made the
question disappear rather than answered it, and is the smaller change. Rejected because it cannot
reach the case the state exists for: the id and the mark are there because *the host's* answer may
never arrive, and a press that has already returned unsettled holds no lock, yet it is that press's
id and mark a later answer must not clear. (The blocking objection people reach for first does not
apply, and is worth naming so nobody re-derives it: presses of one Job go to one host, so they
already serialise on that host's connection lock across the whole dispatch.)

**Refcount instead of identity** — one integer per Job and per cutter, incremented before the
dispatch and decremented by whoever finishes — was rejected for the marks because a poll clears
them wholesale on the cutter's own authority ("it would take a Job right now"), and a decrement
arriving after that clear consumes a *later* press's count. That is #290 again by a third route.
Identity makes a stale clear a no-op instead. The in-doubt entry has a third party of its own —
pruning at `ID_RETENTION` and eviction at the capacity cap can replace an entry while a dispatch is
in flight — so the clear tests two things: that no press in flight is still unanswered, and that
one of those answers named the id the entry actually holds. Identity again is what makes an answer
about a replaced id a no-op.

## Consequences

An answer that settles one press does not clear the Job's in-doubt entry while a sibling press is
still in flight and unanswered, so the entry can outlive the answer that would previously have
removed it. A press that ends without a settling answer keeps it standing indefinitely, which is
the point — something may be cutting under that id. The next press then reuses the id and the host
reads it as the Job it already has, which is reported as a duplicate rather than cut again: the
safe direction, and the one #121 asked to be visible rather than silent. Nothing extends past
`ID_RETENTION`, which is still where an id stops being one a retry can use.
