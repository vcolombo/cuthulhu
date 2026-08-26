<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# The close guard answers for every cut this desktop started, and quitting leaves host-owned Jobs running

The window-close guard asked whether the *aimed* cutter might be cutting, so a Job dispatched to a
Cut Host and then aimed away from — or aimed away from by forgetting the host, or by disconnecting
— closed the window with no warning at all, though the desktop was still holding the record of it
(#158). The guard now asks the question it was there for: is any cut this desktop started still
outstanding, whichever cutter it is on, and with no cutter aimed at all.

Widening it forces a matching decision about what "quit anyway" then does, because the aim was
also what `force_quit` cancelled. It cancels local motion and nothing else: the local cutter's
transport is owned by this process and dies with it, so a Job left mid-motion there could never be
resumed or stopped, while a Cut Host owns its Jobs by design and keeps cutting whether this desktop
is running or not — which is the same rule `disconnect` already follows for a remote cutter. So
quitting stops what only this process can stop, and leaves what the host owns to the host.

## Considered options

**Cancel every outstanding remote Job on quit** was rejected twice over: it reaches into Jobs a
Cut Host owns, on cutters the operator is not looking at, and it would make quitting destroy work
that survives a quit today. A cancel of a remote Job is worth having as its own deliberate,
addressed action with an acknowledgement to wait for; it is not a side effect of closing a window.

**Keep cancelling whatever happens to be aimed** was rejected as the same incoherence #158 is
about: it cancels a host-owned Job when the operator happens to be looking at it and leaves an
identical one running when they are not, and the aim is not a statement about either.

## Consequences

An operator who quits with a remote Job outstanding is warned and then leaves it cutting, so the
warning has to say that rather than implying the quit stops it.

The clear has to be as wide as the question. A guard that asks about every cutter needs a poll that
hears from every cutter, or a mark on one the operator has aimed away from stands for the rest of
the session: not a window that cannot be closed — the prompt always offers the quit — but a prompt
that can never stop being raised, which is how a warning becomes something to click through. So the
device-list poll clears marks for every cutter that says it would take a Job, and forgetting a host
retracts the marks its own answer covered.
