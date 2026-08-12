<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# A cancellable resolver for Cut Host names — design

Date: 2026-08-11
Status: approved (brainstorming complete)

## Purpose

Issue #126 is a trade-off written down rather than a defect: `crates/cut-host/src/client.rs`
bounds how long a caller *waits* on a name resolve (helper thread, channel, `recv_timeout`) and
bounds how many threads that machinery can hold (a 128-slot atomic ceiling plus a per-name
dedup), but it cannot cancel the OS resolver itself. A wedged lookup parks a thread until the OS
returns — or forever — and once the ceiling is reached, a healthy new name is refused because
unrelated names are stuck. Four rounds of review on #125 objected to whichever half the previous
fix had not addressed, which is the shape of a trade-off: any finite ceiling refuses at the
ceiling, and removing it restores unbounded growth. The tension only resolves by making
cancellation possible, and that means dependencies — which #126 asked to have decided
deliberately rather than during a bugfix.

This spec records that decision: **build the cancellable stack, delete the thread machinery.**

## The fact that reshaped the decision

The issue imagined "an async resolver crate", singular. No such crate covers this product's
common path. A Cut Host is dialled as `cuthulhu-pi.local:7878` (`docs/cut-host.md`), which is
mDNS — and [hickory-dns removed its mDNS support during the 0.25 release
cycle](https://github.com/hickory-dns/hickory-dns/releases/tag/v0.25.0-alpha.5); before that it
was experimental and IPv4-only. mDNS is also where resolvers actually wedge ("mDNS on a flaky
network is the ordinary way a resolver wedges" — #109, and the comment above
`resolve_by_deadline`). So the stack is two crates, split along the routing boundary:

- [`mdns-sd`](https://docs.rs/mdns-sd) for `.local` names —
  [`ServiceDaemon::resolve_hostname`](https://docs.rs/mdns-sd/latest/mdns_sd/struct.ServiceDaemon.html)
  answers over a channel from one long-lived daemon thread; a query is a subscription that can
  be stopped, not a syscall that must be waited out. Pure Rust, no runtime dependency.
- [`hickory-resolver`](https://docs.rs/hickory-resolver) for everything else — reads the system
  configuration (resolv.conf / registry, hosts file, search domains) and resolves over its own
  sockets on a tokio runtime, so dropping the lookup future genuinely cancels the I/O.

## Routing

A new module, `crates/cut-host/src/resolve.rs`, takes `resolve_by_deadline(addr, deadline)`
with its current signature and current contract (a `Vec<SocketAddr>` or a `ClientError`, never
blocking past the deadline). `client.rs` is at 744 lines and the resolver was already its most
self-contained region; it moves rather than grows in place. Routing is by shape of the string:

1. **Literal address** (`192.168.1.50:7878`, `[fe80::1]:7878`) → parsed and returned. No
   lookup, no daemon, no runtime — a host paired by IP stays reachable whatever the machine's
   name resolution is doing. Unchanged from today, and the existing test pins it.
2. **Host ends `.local` or `.local.`, case-insensitive** → mdns-sd.
3. **Anything else** → hickory.

Resolved addresses are ordered IPv4-before-IPv6 before the connect loop tries them. mDNS
commonly returns an IPv6 link-local first, and on a network where IPv6 is dead that is the one
address that cannot work; today's code defends against that only by sharing one deadline across
attempts. Ordering removes the failure mode instead of budgeting around it.

## Concurrency model

Two long-lived threads replace zero-to-128 leaked ones:

- **One tokio runtime** (multi-thread flavour, one worker), created lazily in a `OnceLock`,
  living for the process. A unicast lookup is
  `handle.block_on(tokio::time::timeout(remaining, resolver.lookup_ip(host)))` — the timeout
  drops the future, the future's sockets close, nothing is left running. The multi-thread
  flavour (not `current_thread`) is what lets two hosts resolve concurrently: `block_on`
  against a `current_thread` runtime serializes callers, and a second host would queue behind a
  wedged first.
- **The hickory resolver is rebuilt per call.** Building one parses the system configuration —
  cheap — and a desktop runs for days across VPN connects and network moves that rewrite
  resolv.conf. A cached resolver would keep dead nameservers until restart. The redial path
  runs once a second only while a host is unreachable, so per-call parsing costs nothing that
  matters. `// ponytail:` in code names the upgrade: cache keyed on the config file's mtime, if
  a profile ever shows the parse.
- **One mdns-sd `ServiceDaemon`**, created lazily in a `OnceLock`, living for the process; the
  crate watches interface changes itself. A `.local` lookup calls `resolve_hostname`, drains
  the receiver until it has addresses or the deadline passes, then `stop_resolve_hostname` —
  the query is unsubscribed, and no thread is parked anywhere on our side.
- **Failure to create either** (no multicast socket, unreadable resolver config) surfaces as
  `ClientError::Transport` with prose that says what is broken, in the crate's existing error
  voice. No fallback to the old thread machinery — it no longer exists — and no silent retry.

## What is deleted

From `client.rs`: `RESOLVING` (the per-name dedup set), `RESOLVER_THREADS`,
`MAX_RESOLVER_THREADS`, `claim_slot`, the spawn-and-channel machinery, and the `ponytail:`
comment that named this upgrade. With them go two operator-visible refusals — "still being
resolved from an earlier attempt" and the ceiling's "too many host names are stuck" — and the
two tests that pin slot accounting. The cut dialog's once-a-second redial against an
unreachable host now issues a fresh query each poll and cancels it at the deadline; there is
nothing for a repeat attempt to collide with and no count for it to inflate.

## Dependencies, features, and the Pi build

`cut-host` is one crate with two lives: the daemon binary (`cuthulhu-cutd`, cross-compiled to
`aarch64-unknown-linux-gnu` as a required CI check) and the client module (consumed only by
`apps/desktop`). The daemon never resolves a name — it binds. The crate's dependency list is
deliberately short (its `Cargo.toml` justifies `libc` by "already in the tree"), and three new
dependencies for code the daemon cannot reach would break that discipline.

So: a cargo feature `client`, **default-on**, gating `pub mod client`, `mod resolve`, and the
three dependencies — `tokio` (features `rt-multi-thread`, `net`, `time`), `hickory-resolver`,
`mdns-sd`. Default-on means `cargo test --workspace --locked` still builds and runs the client
tests with no flag anywhere. The CI cross build adds `--no-default-features` to its existing
command, so the Pi daemon compiles without any of the three. `Cargo.lock` is committed in the
same change, because CI's `--locked` refuses to rewrite it. The desktop's `Cargo.toml` needs no
change. tokio is already in the workspace's lock via Tauri, so the workspace-level cost is
cut-host's own graph, not a new tree.

## Accepted behaviour divergences

Replacing `getaddrinfo` means no longer inheriting every OS resolver behaviour. Two divergences
are accepted and recorded here so nobody re-files them as regressions:

- **Single-label names** (`cuthulhu-pi:7878`, no suffix): macOS's resolver would try mDNS for
  these; the new stack routes them to hickory, which applies DNS search domains only.
  `docs/cut-host.md` gains one line: address a host as `name.local:port` or `ip:port`.
- **Unicast-`.local`** (enterprise networks serving `.local` from a DNS server, violating RFC
  6762): stops resolving, because `.local` now goes to multicast only. Consistent with the RFC
  and with where the platform vendors have gone.

Hosts-file names, router-DNS names (`pi.lan`), and search-domain names keep working through
hickory's system configuration support.

## Testing

- **Routing is a pure function** over the address string — unit tests cover literals (v4, v6
  brackets), `.local` in both casings and with a trailing dot, and unicast names, no network
  involved.
- **Existing tests survive**: the literal-address test (route 1 unchanged), the `.invalid`
  test (now exercising hickory's NXDOMAIN instead of `getaddrinfo`'s), and both blackhole
  connect-timeout tests.
- **The cancellation bound is proven, not assumed**: an internal seam takes a hickory
  `ResolverConfig`, and a test points it at the conventional black-holed address
  (`10.255.255.1`) and asserts the call returns within its deadline.
- **mDNS no-answer smoke test**: resolving `nonexistent-<nonce>.local` must return by the
  deadline whether the runner's multicast works (query sent, nothing answers) or is blocked
  (daemon errors) — it asserts the bound, which is the property this spec exists to buy.
- **mDNS positive path** cannot be trusted on CI (runner multicast is flaky), so it is an
  `#[ignore]`d loopback register-and-resolve test plus an entry in
  `apps/desktop/MANUAL-CHECKLIST.md`: pair a real Pi by `cuthulhu-pi.local`, confirm the
  lookup, and confirm a wedged network times out fast and the host is reachable again
  afterwards — with device and date, per that file's convention.

## Out of scope

- Caching a host's last-good address (a mitigation the stack makes unnecessary).
- Service *discovery* — browsing for Cut Hosts on the network is #52/#42 territory; this spec
  only resolves names the operator typed.
- Any change to the daemon, the protocol, or pairing flows.
