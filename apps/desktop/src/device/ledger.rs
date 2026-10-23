// SPDX-License-Identifier: GPL-3.0-or-later
//! Which press owns an in-doubt id, a dispatch registration, and a close-guard mark.
//!
//! One owner, because the three maps share a lock order and an answer about one press must not
//! clear what another press is still using (#290). Nothing outside this module takes those locks.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use driver_core::HostId;

/// Whether an id written at `written` is still one a retry can be sent under.
///
/// The one place this desktop decides that, read by `reserve_dispatch_id` — which prunes by it —
/// and by `retry_would_reuse`, which is how the sentence above learns whether the retry it offers
/// is one the next press will actually make. Two copies of the rule would be two policies, and the
/// promise #286 removed was exactly what a disagreement between them produces.
///
/// Aligned to the host's own `ID_RETENTION` rather than a number of this side's invention: past it
/// the host has forgotten the id, so reusing it protects nothing and only hides that it protects
/// nothing. The two windows do not coincide — this ages from when the id was written, while a
/// host re-stamps an id every time a duplicate arrives — and neither side's cap is in it at all.
fn still_reusable(written: std::time::Instant, now: std::time::Instant) -> bool {
    now.saturating_duration_since(written) < cut_host::host::ID_RETENTION
}

/// How many Jobs may be held in doubt at once.
///
/// A backstop against a session that dispatches thousands of distinct designs to hosts that never
/// answer, not a policy anyone should reach: entries clear on a settling answer, and expire on the
/// host's own `ID_RETENTION` — though not in step with the host's own memory of the id, for the
/// reasons on `in_doubt`. The oldest is evicted, which is the one a retry is least likely to name.
pub(super) const MAX_JOBS_IN_DOUBT: usize = 256;

/// One dispatchable Job as this desktop identifies it: which cutter, on which host, carrying
/// what. Two presses of Cut that agree on all three are the same Job — which is what makes the
/// second one a candidate for being a retry of the first.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub(super) struct JobKey {
    pub(super) host: HostId,
    pub(super) device: String,
    pub(super) digest: u64,
}

/// One press of Cut, as something the state it writes can be attributed to.
///
/// Both pieces of per-Job dispatch state are shared by presses — the id a retry goes out under,
/// and the mark that holds the window — and neither had room for two, so an answer about one press
/// cleared what another was still using (#290). A press retracts only the mark carrying its own
/// id; the Job's in-doubt entry is shared, and what a press may do to it is `settle_dispatch`.
/// The alternatives, and why a count is not one of them, are in
/// `docs/adr/0001-a-dispatch-attempt-owns-what-it-wrote.md`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct AttemptId(u64);

impl AttemptId {
    /// Distinct for the life of the process, which is the life of every set that holds one. The
    /// nonce `reserve_dispatch_id` puts in a `DispatchId` counts something else — fresh ids minted,
    /// not presses made — and the two deliberately do not meet: this identity is private to the
    /// desktop, and that one goes on the wire.
    pub(super) fn next() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        AttemptId(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
}

/// Presses of one Job, grouped by the wire id each press is dispatching.
///
/// The id is the settlement boundary. Two presses can share it, while expiry or capacity pruning
/// can mint a replacement with a press under the old one still in flight. An answer about the old
/// id must not block or clear the replacement; an answer about the current id must wait for every
/// press already dispatching *that id*.
///
/// A group remains after its last press returns only when one returned without a settling answer.
/// The next press of that id is then the retry: its settling answer resolves the group and can
/// clear the in-doubt entry. That preserves #121's second intentional Cut while keeping #290's
/// concurrent press from losing the id its retry needs.
#[derive(Default)]
struct Presses {
    by_dispatch: HashMap<cut_host::protocol::DispatchId, DispatchGroup>,
}

/// The presses that named one DispatchId, in the four states an answer is judged against.
#[derive(Default)]
struct DispatchGroup {
    /// Presses that have begun and not returned. Dropped by `Dispatching`, so it empties itself.
    in_flight: HashSet<AttemptId>,
    /// Presses whose answer arrived, retained until the group goes — an answer outlives its guard.
    answered: HashSet<AttemptId>,
    /// Presses that returned with nothing settled. Emptied only by a settling answer from a press
    /// in `retries`, which is what makes an unresolved outcome one press's to resolve.
    unsettled: HashSet<AttemptId>,
    /// Presses that began with `in_flight` empty and something left `unsettled`: a retry that can
    /// resolve those outcomes, rather than a concurrent sibling or a Job's first press.
    retries: HashSet<AttemptId>,
}

/// A press counted as dispatching for exactly as long as it is in flight.
///
/// A guard rather than a pair of calls because the registration is what stops a *sibling* press's
/// answer from clearing this one's id, so an early return or a panic that skipped the removal
/// would leave a Job no answer could settle, and every press until `ID_RETENTION` expires the
/// entry read as a retry rather than cut.
///
/// Drop records a press that returned without a settling answer rather than forgetting it. A
/// future press that begins after the group has no in-flight presses is its retry; only that
/// retry's settling answer removes the unresolved record. A settled answer is kept until the
/// group is removed, because a sibling can be the press that finally proves every dispatch of
/// this id has been answered.
#[must_use = "the press stops counting as dispatching as soon as this is dropped"]
pub(super) struct Dispatching<'a> {
    ledger: &'a Ledger,
    key: JobKey,
    pub(super) attempt: AttemptId,
    dispatch_id: cut_host::protocol::DispatchId,
}

impl Drop for Dispatching<'_> {
    fn drop(&mut self) {
        // `into_inner` rather than `unwrap`, as everywhere this repo locks in a `Drop`: a panic
        // here while unwinding aborts the process, so the one path that most needs the press
        // deregistered would instead take the editor down.
        let mut dispatching = self.ledger.dispatching.lock().unwrap_or_else(|e| e.into_inner());
        let Some(presses) = dispatching.get_mut(&self.key) else {
            debug_assert!(false, "a press's registration went missing while it was in flight");
            return;
        };
        let Some(group) = presses.by_dispatch.get_mut(&self.dispatch_id) else {
            debug_assert!(false, "a dispatch group went missing while its press was in flight");
            return;
        };
        group.in_flight.remove(&self.attempt);
        group.retries.remove(&self.attempt);
        if !group.answered.contains(&self.attempt) {
            group.unsettled.insert(self.attempt);
        }

        if group.in_flight.is_empty() {
            // `dispatching` -> `in_doubt`, the one declared order. A group for an id the Job no
            // longer holds can never be retried and is dead state; a current unresolved group is
            // kept so the next press can be identified as the retry that resolves it.
            let current = self
                .ledger
                .in_doubt
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&self.key)
                .is_some_and(|(held, _)| held == &self.dispatch_id);
            if !current || group.unsettled.is_empty() {
                presses.by_dispatch.remove(&self.dispatch_id);
            }
        }
        if presses.by_dispatch.is_empty() {
            dispatching.remove(&self.key);
        }
    }
}


#[derive(Default)]
pub(super) struct Ledger {
    /// The dispatch id a Job's next press goes out under while an entry for it stands, by the Job it
    /// names.
    ///
    /// The host deduplicates on the dispatch id precisely so a retry after a dropped reply
    /// cannot cut the same material twice — but only the sender knows whether a given press of
    /// Cut *is* that retry, and nothing in the Job says so: a retry and a second sheet of the
    /// same design are byte-identical. What separates them is this entry: while a Job has one, the
    /// next press reuses the id in it instead of minting a fresh one.
    ///
    /// Written before the request goes out, and dropped by an answer that settles what the host did:
    /// `Accepted` means it has the Job, a refusal means it started nothing. A dropped reply settles
    /// neither, and since #283 neither does a reply the request could not use — it arrived, so
    /// nothing was lost, but it says nothing about whether the Job began.
    ///
    /// "Settles" is the client's reading of the reply, not a proof about the host. An `Accepted` is
    /// believed without checking which dispatch it names (#285), and past the bounds below the id it
    /// holds may mean nothing to the host any more. Deduplicating by memory is best-effort on both
    /// sides, and nothing here upgrades it.
    ///
    /// So an entry does not mean "outcome unknown", and a dispatch that never left this machine is
    /// the case that shows the difference: its outcome is known — nothing started — and the entry
    /// stays anyway, because clearing it buys nothing. A host that was never reached cannot have
    /// seen the id either, so the next Cut sending it again is cut normally rather than
    /// deduplicated away (see `execute_cut`). Whose entry it is *is* answerable, by `dispatching`
    /// below; it is only that the answer changes nothing here. Kept, in other words, wherever
    /// letting go would buy nothing or risk a press's retry, which is a weaker condition than
    /// doubt.
    ///
    /// Beyond those two answers an entry leaves only by expiry or by making room, below. An earlier
    /// version aged them out after fifteen minutes, which was the
    /// wrong fix for a dispatch nobody revisits: inside the host's window, time cannot show that
    /// the operator loaded fresh material, so expiring early mints a new id for a Job the host
    /// still remembers under the old one and cuts the design twice. What makes an entry nobody
    /// revisits harmless is that the next Cut is *told* it was read as a retry
    /// (`CutStarted::duplicate`), and that a settling answer clears the entry, so pressing Cut
    /// again really does cut (#121).
    ///
    /// The one honest expiry is `ID_RETENTION`, the host's own constant: an hour after this entry
    /// was written, holding it only pretends to a protection that may well have lapsed. Nothing on
    /// either side can prevent a re-cut past that point — which is the true limit of deduplicating
    /// by memory, not something a client-side rule can close.
    ///
    /// "May well", because sharing the constant does not make the two windows the same window. This
    /// entry ages from when the id was written; a host's `Admission` re-stamps an id every time a
    /// duplicate arrives, so a Job retried near the hour can be remembered there after it has
    /// expired here. And two capacity bounds cut in ahead of the hour on both sides, neither
    /// controlling the other's: this map evicts its oldest entry at `MAX_JOBS_IN_DOUBT`, a host
    /// forgets its oldest accepted id at `MAX_REMEMBERED_IDS`. So an entry can outlive the host's
    /// memory of its id and an id can outlive this entry, in either direction. All of it is a
    /// backstop against exhaustion rather than a policy, which is why the numbers are set where
    /// reaching them is itself the unusual event.
    ///
    /// **Lock order:** taken alone, or last — never with `hosts` or a `HostConnection` acquired
    /// while holding it, and under `dispatching` where a clear has to ask about both at once
    /// (`settle_dispatch`).
    in_doubt: Mutex<HashMap<JobKey, (cut_host::protocol::DispatchId, std::time::Instant)>>,
    /// Presses of Cut dispatching each Job, grouped by the wire id they use.
    ///
    /// Ownership, which `in_doubt` cannot express on its own: two presses of the same Job can be
    /// in flight at once, sharing the one entry, and an answer that settles one of them says
    /// nothing about the other. Clearing the entry on the strength of that answer took away the
    /// record the other press's retry sends under, so the next press minted a name the host had
    /// never seen (#290). `Presses` retains both settling and non-settling outcomes across guard
    /// drops, and scopes them to the DispatchId they concern.
    ///
    /// **Lock order:** before `in_doubt`, never after; and like `in_doubt`, never with `hosts` or
    /// a `HostConnection` acquired while holding it — `settle_dispatch` runs inside `with_host`,
    /// which means this is taken with a connection already held.
    dispatching: Mutex<HashMap<JobKey, Presses>>,
    /// Remote dispatches this desktop started and has not since seen end, by cutter and by the
    /// press that started each one.
    ///
    /// The cache above cannot answer for the window that matters most: a dispatch accepted a
    /// moment ago has not been polled yet, so the newest thing anyone heard is the `Idle` from
    /// before it — and the window-close guard would wave the operator past a cut it just started.
    /// This says "we started something and nothing has told us it ended" without dialling.
    ///
    /// Per press rather than a boolean per cutter, because a press that learns the host started
    /// nothing retracts its own mark, and one press's refusal says nothing about another press —
    /// of this Job or of any other on that cutter — that was accepted and is cutting right now
    /// (#290). A poll that finds the cutter free clears every mark on it at once, which is not the
    /// same act: it is the cutter itself saying it would take another Job, and no press outranks
    /// that.
    ///
    /// Cleared by any poll that hears a cutter say it would take a Job — `status` for the aimed
    /// one, `list_devices` for every cutter on every paired host — and by forgetting the host.
    /// Between those it holds one id per press, so a session that dispatches without ever polling
    /// again grows this by one entry per press; the numbers are an operator's finger, not a loop.
    remote_dispatched: Mutex<HashMap<(HostId, String), HashSet<AttemptId>>>,
}

impl Ledger {
    pub(super) fn a_press_is_dispatching(&self) -> bool {
        self.dispatching
            .lock()
            .unwrap()
            .values()
            .any(|presses| presses.by_dispatch.values().any(|group| !group.in_flight.is_empty()))
    }

    /// The same question asked of one host, for the verb that is about to discard its token.
    pub(super) fn a_press_is_dispatching_to(&self, id: &HostId) -> bool {
        self.dispatching.lock().unwrap().iter().any(|(key, presses)| {
            &key.host == id && presses.by_dispatch.values().any(|group| !group.in_flight.is_empty())
        })
    }

    /// Register this press as dispatching `key`, and choose the id it goes out under.
    ///
    /// One call rather than two, because the order is the invariant: a press that chose an id
    /// before it was counted could have a sibling's settling answer land in the gap, clear the
    /// entry it is about to dispatch under, and leave a later retry sending a name the host has
    /// never seen. Two statements can be swapped by an edit; one cannot.
    ///
    /// The `bool` is `reserve_dispatch_id`'s: `true` means this call is what recorded the entry.
    pub(super) fn begin_dispatch(&self, key: &JobKey) -> (Dispatching<'_>, cut_host::protocol::DispatchId, bool) {
        let attempt = AttemptId::next();
        // `dispatching` first and `in_doubt` under it — the order stated on both fields, and not
        // the order they are declared in — held across the reservation so a sibling's answer cannot
        // run between the two halves and find nobody dispatching.
        let mut dispatching = self.dispatching.lock().unwrap();
        let (dispatch_id, first_attempt) = self.reserve_dispatch_id(key);
        // `in_doubt` prunes and evicts ids independently. An unresolved group for an id it no
        // longer holds cannot be retried, so keeping it would make `dispatching` grow past the map
        // whose bound it mirrors. Active groups stay until their guards drop; inactive replaced
        // groups go here.
        {
            let in_doubt = self.in_doubt.lock().unwrap();
            dispatching.retain(|job, presses| {
                presses.by_dispatch.retain(|wire_id, group| {
                    !group.in_flight.is_empty()
                        || in_doubt.get(job).is_some_and(|(held, _)| held == wire_id)
                });
                !presses.by_dispatch.is_empty()
            });
        }
        let group = dispatching
            .entry(key.clone())
            .or_default()
            .by_dispatch
            .entry(dispatch_id.clone())
            .or_default();
        // A press beginning after every earlier press of this id returned is the retry that can
        // resolve their unconfirmed outcomes. A concurrent sibling is not: its answer belongs only
        // to itself, which is the distinction #290 was missing.
        if group.in_flight.is_empty() && !group.unsettled.is_empty() {
            group.retries.insert(attempt);
        }
        group.in_flight.insert(attempt);
        drop(dispatching);
        (
            Dispatching {
                ledger: self,
                key: key.clone(),
                attempt,
                dispatch_id: dispatch_id.clone(),
            },
            dispatch_id,
            first_attempt,
        )
    }

    /// Account for what this press was told, and clear only the DispatchId that answer concerned.
    ///
    /// A concurrent sibling under the same id still blocks: its reply may be lost, and the entry
    /// is what its retry would send (#290). A retry that began after every earlier press returned
    /// resolves those unconfirmed outcomes when it gets a settling answer, preserving the second
    /// intentional Cut #121 requires. Groups under replaced ids neither block nor clear the current
    /// one.
    pub(super) fn settle_dispatch(&self, key: &JobKey, id: &cut_host::protocol::DispatchId, attempt: AttemptId) {
        // Both locks, `dispatching` then `in_doubt` as both fields state: asking and clearing have
        // to be one step, or a press beginning between them joins an entry this call is about to
        // delete — and its retry then goes out under a name the host has never seen.
        let mut dispatching = self.dispatching.lock().unwrap();
        let mut in_doubt = self.in_doubt.lock().unwrap();
        let Some(group) = dispatching
            .get_mut(key)
            .and_then(|presses| presses.by_dispatch.get_mut(id))
        else {
            return;
        };
        group.answered.insert(attempt);
        if group.retries.contains(&attempt) {
            group.unsettled.clear();
        }
        let every_press_answered = group.in_flight.iter().all(|p| group.answered.contains(p));
        let current_id = in_doubt.get(key).is_some_and(|(held, _)| held == id);
        if every_press_answered && group.unsettled.is_empty() && current_id {
            in_doubt.remove(key);
        }
    }

    pub(super) fn mark_dispatched(&self, host: &HostId, device: &str, attempt: AttemptId) {
        self.remote_dispatched
            .lock()
            .unwrap()
            .entry((host.clone(), device.to_owned()))
            .or_default()
            .insert(attempt);
    }

    /// Retract this press's mark, having learned the host started nothing for it.
    ///
    /// Only this press's: the mark is what raises the quit prompt, and a refusal answers for the
    /// dispatch it refused rather than for another press that was accepted and is cutting (#290).
    /// A cutter's own "I would take a Job now" is the answer that clears them all, in `status` and
    /// `list_devices`.
    pub(super) fn unmark_dispatched(&self, host: &HostId, device: &str, attempt: AttemptId) {
        let mut dispatched = self.remote_dispatched.lock().unwrap();
        let key = (host.clone(), device.to_owned());
        if let Some(presses) = dispatched.get_mut(&key) {
            presses.remove(&attempt);
            // An empty set and no entry mean the same thing, so only one spelling is stored: the
            // map holds a cutter for exactly as long as some press holds it.
            if presses.is_empty() {
                dispatched.remove(&key);
            }
        }
    }

    /// Drop exactly the marks named, having learned the presses that wrote them started nothing.
    ///
    /// The retraction `forget` makes, and deliberately narrower than "every mark for that host":
    /// what makes them stale is the host's own answer that it is idle, and a dispatch accepted
    /// after that answer is not covered by it. That press keeps its mark and the operator keeps the
    /// warning — which is all this desktop can still offer, having just discarded the token that
    /// could have stopped it.
    pub(super) fn retract_marks(&self, known: &[((HostId, String), AttemptId)]) {
        for ((host, device), attempt) in known {
            self.unmark_dispatched(host, device, *attempt);
        }
    }

    /// Every mark this desktop is holding for `host`, as the presses that wrote them.
    pub(super) fn marks_for(&self, host: &HostId) -> Vec<((HostId, String), AttemptId)> {
        self.remote_dispatched
            .lock()
            .unwrap()
            .iter()
            .filter(|((h, _), _)| h == host)
            .flat_map(|(key, presses)| presses.iter().map(|p| (key.clone(), *p)).collect::<Vec<_>>())
            .collect()
    }

    /// The id this Job goes out under — the one its unsettled dispatch already used, or a fresh
    /// one — recorded as in doubt in the same breath. `true` means this call is what recorded it,
    /// which is what tells a caller that never reached the host that the entry it would be
    /// discarding is its own and not an *earlier* dispatch's. Read rather than acted on:
    /// `execute_cut` uses it only to decide what the operator is told, because a never-reached
    /// dispatch's entry is harmless to leave (see the comment there).
    ///
    /// Get-and-record under one lock, because the gap between them is a Cut pressed twice: both
    /// presses found nothing, minted an id each, and the second overwrote the record of the first
    /// — so if the first had reached the host and lost its reply, the retry that should have been
    /// recognised went out under a name the host had never seen, and cut the material again.
    ///
    /// The nonce is wall-clock rather than a counter because a counter restarts at zero with the
    /// process while the host remembers ids for an hour — a second session would mint ids the host
    /// already knows and have its cuts silently deduplicated away.
    pub(super) fn reserve_dispatch_id(&self, key: &JobKey) -> (cut_host::protocol::DispatchId, bool) {
        let now = std::time::Instant::now();
        let mut in_doubt = self.in_doubt.lock().unwrap();

        // Entries the host can no longer recognise are not retries any more, whatever they once
        // were, and `still_reusable` is where that is decided — see there for why the boundary is
        // the host's own constant. This is *not* the fifteen-minute expiry that was removed; that
        // one discarded protection while it still existed.
        in_doubt.retain(|_, (_, written)| still_reusable(*written, now));
        // Looked up before anything is evicted, so the cap can never discard the very entry this
        // call is here to reuse.
        if let Some((id, _)) = in_doubt.get(key) {
            return (id.clone(), false);
        }

        // A backstop, not a policy: entries are per Job, so a long session cutting many different
        // designs is what grows this, and the oldest is the one a retry is least likely to name.
        // Room is made *before* the insert, so the map settles at the cap rather than one past it.
        while in_doubt.len() >= MAX_JOBS_IN_DOUBT {
            let Some(oldest) =
                in_doubt.iter().min_by_key(|(_, (_, written))| *written).map(|(k, _)| k.clone())
            else {
                break;
            };
            in_doubt.remove(&oldest);
        }
        // The counter is what makes two attempts in the same clock tick distinct; the clock is
        // what keeps this run's ids clear of the previous run's. Neither alone is enough.
        static ATTEMPT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        // The cutter's id is truncated because it is the one part with no length of its own: a
        // serial port enumerated by path can be most of a `/dev/serial/by-id/...` string, and the
        // host refuses an id longer than it will remember — which would have refused *every*
        // dispatch to that cutter. The digest already identifies the Job exactly; the text is for
        // whoever is reading a log.
        let device: String = key.device.chars().take(32).collect();
        let id = cut_host::protocol::DispatchId(format!(
            "{}-{:016x}-{}-{}",
            device,
            key.digest,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
            ATTEMPT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        in_doubt.insert(key.clone(), (id.clone(), now));
        (id, true)
    }

    /// Whether a press made now would send this Job out under the id an unsettled dispatch already
    /// used, which is the only thing that gives a repeated Cut a chance of being recognised rather
    /// than cut again.
    ///
    /// The retention rule lives here and `reserve_dispatch_id` prunes by it, so the sentence an
    /// unconfirmed dispatch carries and the id the next press actually mints answer the same
    /// question. A second copy of that rule would be a second policy, and the two disagreeing is
    /// exactly how a promise becomes false: an entry can cross `ID_RETENTION` while the call
    /// offering the retry is still in flight, and `contains_key` alone would offer to reuse an id
    /// the next press is about to prune (#286).
    ///
    /// `now` is a parameter because the boundary is the whole of the behaviour, and a caller that
    /// can only pass the present cannot ask about either side of it.
    pub(super) fn retry_would_reuse(&self, key: &JobKey, now: std::time::Instant) -> bool {
        self.in_doubt
            .lock()
            .unwrap()
            .get(key)
            .is_some_and(|(_, written)| still_reusable(*written, now))
    }


    pub(super) fn any_mark(&self) -> bool {
        !self.remote_dispatched.lock().unwrap().is_empty()
    }

    pub(super) fn clear_cutter(&self, host: &HostId, device: &str) {
        self.remote_dispatched
            .lock()
            .unwrap()
            .remove(&(host.clone(), device.to_owned()));
    }

    pub(super) fn clear_cutters<'a>(&self, host: &HostId, devices: impl IntoIterator<Item = &'a str>) {
        let mut dispatched = self.remote_dispatched.lock().unwrap();
        for device in devices {
            dispatched.remove(&(host.clone(), device.to_owned()));
        }
    }

    #[cfg(test)]
    pub(super) fn jobs_in_doubt(&self) -> usize {
        self.in_doubt.lock().unwrap().len()
    }

    #[cfg(test)]
    pub(super) fn held_id(&self, key: &JobKey) -> Option<cut_host::protocol::DispatchId> {
        self.in_doubt.lock().unwrap().get(key).map(|(id, _)| id.clone())
    }

    #[cfg(test)]
    pub(super) fn holds(&self, key: &JobKey) -> bool {
        self.in_doubt.lock().unwrap().contains_key(key)
    }

    #[cfg(test)]
    pub(super) fn written_at(&self, key: &JobKey) -> Option<std::time::Instant> {
        self.in_doubt.lock().unwrap().get(key).map(|(_, written)| *written)
    }

    #[cfg(test)]
    pub(super) fn forget(&self, key: &JobKey) {
        self.in_doubt.lock().unwrap().remove(key);
    }

    /// Plant an id the ledger did not mint, so a test can show a stale answer leaving the current
    /// one alone.
    #[cfg(test)]
    pub(super) fn replace_held(&self, key: &JobKey, id: cut_host::protocol::DispatchId) {
        self.in_doubt
            .lock()
            .unwrap()
            .insert(key.clone(), (id, std::time::Instant::now()));
    }

    #[cfg(test)]
    pub(super) fn no_press_registered(&self) -> bool {
        self.dispatching.lock().unwrap().is_empty()
    }
}
