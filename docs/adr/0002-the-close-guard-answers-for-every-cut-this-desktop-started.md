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
device-list poll clears marks for every cutter that says it would take a Job. A dispatch writes its
mark only after it owns the same host-connection lock as that poll, so an idle answer precedes the
mark or follows the request; it can never clear a request still queued behind it.

A close is a commitment, not an answer. Between choosing an id and reaching the host's connection a
press has written nothing, so it is invisible to any mark, and an async command can still cross
into its dispatch after the guard has let the window go — starting a Job that outlives the process
that started it. So the guard's question and the close are one step: the decision is taken while a
send cannot begin, and a press that reaches the connection afterwards sends nothing. "Quit anyway"
waits briefly for a send already in progress rather than forever, because a prompt whose answer
hangs behind a timing-out Pi is its own failure; past that the send may land, which is what the
prompt already says about a Job sent to a Cut Host.

Free is two facts, not one. A cutter is only free to have its warning dropped when nothing claims
it *and* it says it would take another Job: disconnected, errored, and stopped-without-confirmation
are all unclaimed and none of them are free. Forgetting a reachable host therefore refuses while any
cutter is not free, and what `force` may pass is decided by whether anything can recover the state:

- **Claimed** — a Job in flight, or a dispatch on its way to the manager. Refused whatever the
  operator insists, as it always was: the host is answering, so `cancel` reaches the blade.
- **Stopped where nothing saw it stop** — refused too, because the host's own `Reconnect` clears it.
  A refusal with a verb one click away is not something to force past.
- **Disconnected or errored** — refused on the ordinary path and passable with `force`, because a
  cutter whose hardware is gone answers every reconnect with the same fault and has no cut to
  cancel. Without that escape the host row could never be removed, which is worse than the warning
  it would have kept.

Forgetting a host retracts the marks its answer covered. A dispatch that gets the connection after
that answer keeps its mark even though the host's cancel route is gone, and the HostId remains
claimed for the process lifetime so an idle newly-paired host cannot inherit and erase the warning.

A prevented close also needs a positively known listener. Tauri reports a successful emit with
zero listeners, so the webview acknowledges the listener after registration and clears that
readiness before teardown; outside that lifetime the requested close proceeds.
