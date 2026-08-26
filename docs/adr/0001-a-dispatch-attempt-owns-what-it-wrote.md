<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# A dispatch attempt owns what it wrote

Two presses of Cut for the same Job can be dispatching at once, and both share the two pieces of
state that stand between an unconfirmed dispatch and material cut twice: the in-doubt id a retry
goes out under, and the mark that tells the window-close guard something was started. Both were
keyed to the Job alone, so an answer about one press cleared what the other press was still using
(#290). We gave a press an identity, and made every clear an attributed one: a press clears the
mark it wrote and no other, and it clears the Job's in-doubt entry only when no other press is
dispatching under it — the last one out.

## Considered options

**Serialise presses per Job** — a per-Job lock held across the dispatch — would have made the
question disappear rather than answered it, and is the smaller change. Rejected because the lock
would have to be held across the network call, so a second press would block for as long as a
host takes to answer or to time out, and the desktop's whole reason for reading status without
dialling is that a press must not be able to freeze the UI. It also answers only the presses this
process makes: the mark and the id exist because *the host's* answer may never arrive, and a lock
here cannot make that case go away.

**Refcount instead of identity** — one integer per Job and per cutter, incremented before the
dispatch and decremented by whoever finishes — was rejected for the marks because the status poll
clears them wholesale on the cutter's own authority ("it would take a Job right now"), and a
decrement arriving after that clear consumes a *later* press's count. That is #290 again by a
third route. Identity makes a stale clear a no-op instead. The in-doubt entry has no such third
party, so the question there is only "is another press in flight", and a set of the presses that
are answers it directly.

## Consequences

An answer that settles one press no longer clears the Job's in-doubt entry while a sibling press
is still in flight, so the entry can outlive the answer that would previously have removed it. The
next press then reuses the id and the host reads it as the Job it already has, which is reported
as a duplicate rather than cut again — the safe direction, and the one #121 asked to be visible
rather than silent. Nothing extends past `ID_RETENTION`, which is still where an id stops being
one a retry can use.
