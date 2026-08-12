// SPDX-License-Identifier: GPL-3.0-or-later
//! Turning a typed Cut Host address into socket addresses, cancellably.
//!
//! Three shapes, three fates: a literal address needs no lookup at all; a `.local` name is
//! mDNS and goes to a daemon whose queries can be stopped; everything else is unicast DNS on
//! a runtime whose lookups can be dropped. Nothing here parks a thread on the OS resolver —
//! that machinery, its ceiling, and its leak were deleted with #126.

use std::net::SocketAddr;

use crate::client::ClientError;

/// Where an address string must be sent to become socket addresses.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Route {
    Literal(SocketAddr),
    Mdns { host: String, port: u16 },
    Dns { host: String, port: u16 },
}

/// Classify `addr` (`host:port`) by shape. See the module doc for why the three shapes part
/// ways here rather than all being handed to one resolver.
pub(crate) fn route(addr: &str) -> Result<Route, ClientError> {
    // `to_socket_addrs` would answer a literal from the string itself too, but only after
    // being handed to the machinery below — parsing first keeps a host paired by IP clear of
    // every resolver, which `docs/cut-host.md`'s documented setup relies on.
    if let Ok(literal) = addr.parse::<SocketAddr>() {
        return Ok(Route::Literal(literal));
    }
    let malformed =
        || ClientError::Transport(format!("`{addr}` is not a host:port address"));
    let (host, port) = addr.rsplit_once(':').ok_or_else(malformed)?;
    let port: u16 = port.parse().map_err(|_| malformed())?;
    if host.is_empty() {
        return Err(malformed());
    }
    // One trailing dot is the explicit DNS root, not part of the name; mdns-sd and hickory
    // each want their own spelling, so the route carries the bare lower-cased form.
    let host = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
    if host.ends_with(".local") {
        Ok(Route::Mdns { host, port })
    } else {
        Ok(Route::Dns { host, port })
    }
}

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use hickory_resolver::Resolver;
// Only the test seam takes a caller-supplied configuration; real callers reach hickory
// through the system configuration in `resolve_dns`, so these ride the same cfg as the seam.
#[cfg(test)]
use hickory_resolver::config::ResolverConfig;
#[cfg(test)]
use hickory_resolver::net::runtime::TokioRuntimeProvider;

use mdns_sd::{HostnameResolutionEvent, ServiceDaemon};

/// One daemon for the process, like the runtime above: it owns the multicast sockets and
/// watches interface changes itself, and a query against it is a subscription that can be
/// stopped — which is the entire upgrade over handing `.local` to the OS resolver.
static DAEMON: OnceLock<Result<ServiceDaemon, String>> = OnceLock::new();

fn daemon() -> Result<&'static ServiceDaemon, ClientError> {
    DAEMON
        .get_or_init(|| ServiceDaemon::new().map_err(|e| e.to_string()))
        .as_ref()
        .map_err(|e| {
            ClientError::Transport(format!("multicast DNS is unavailable on this machine ({e})"))
        })
}

/// One live query per name — not to bound anything, but because mdns-sd keeps a single
/// listener per hostname: a second `resolve_hostname` for the same name overwrites the first
/// caller's channel, and either caller's `stop_resolve_hostname` tears down the other's query.
/// Serializing per name lets both callers answer, where the old machinery refused one.
/// ponytail: entries are never pruned — a few bytes per distinct name ever dialled, bounded by
/// paired hosts and pairing-dialog typos; prune on last-caller-out if that ever matters.
static LOCAL_QUERIES: Mutex<BTreeMap<String, Arc<Mutex<()>>>> = Mutex::new(BTreeMap::new());

/// `host`'s addresses over mDNS, or a failure once `deadline` passes. Never blocks past the
/// deadline: the query is unsubscribed on every exit, so a wedged network costs the wait and
/// nothing else — no thread, no slot, no ceiling.
pub(crate) fn resolve_mdns(
    host: &str,
    port: u16,
    deadline: Instant,
) -> Result<Vec<SocketAddr>, ClientError> {
    let daemon = daemon()?;
    // mdns-sd wants the fully-qualified spelling; `route` stripped the trailing dot off.
    let fqdn = format!("{host}.");
    let gate = LOCAL_QUERIES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(fqdn.clone())
        .or_default()
        .clone();
    // Waiting a turn spends this caller's own budget — everything below recomputes what is
    // left — so a queue behind a slow same-name query still returns by this caller's deadline.
    let _one_query_per_name = gate.lock().unwrap_or_else(|e| e.into_inner());

    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(ClientError::Transport(format!(
            "`{host}` was not answered over mDNS in time"
        )));
    }
    let events = daemon
        .resolve_hostname(&fqdn, Some(remaining.as_millis().try_into().unwrap_or(u64::MAX)))
        .map_err(|e| ClientError::Transport(format!("could not query `{host}` over mDNS ({e})")))?;

    let mut found: Vec<SocketAddr> = Vec::new();
    // First answer wins. mDNS responders send their full address set in one shot, and the
    // dialler beyond this wants addresses to try, not a census — waiting out the deadline to
    // collect stragglers would spend the connect budget on completeness nobody asked for.
    while found.is_empty() {
        match events.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(HostnameResolutionEvent::AddressesFound(_, addrs)) => {
                found.extend(addrs.iter().map(|scoped| SocketAddr::new(scoped.to_ip_addr(), port)));
            }
            Ok(HostnameResolutionEvent::SearchTimeout(_)) => break,
            Ok(HostnameResolutionEvent::SearchStopped(_)) => break,
            // SearchStarted, AddressesRemoved, and whatever the non_exhaustive enum grows
            // later: none of them is an answer, so keep waiting. SearchStopped breaks above
            // because a stopped query is gone; waiting can only spend the deadline it cannot answer.
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    // Unsubscribe on every path — the receiver going quiet must not leave a live query
    // accumulating answers nobody will read.
    let _ = daemon.stop_resolve_hostname(&fqdn);

    if found.is_empty() {
        Err(ClientError::Transport(format!("`{host}` was not answered over mDNS in time")))
    } else {
        Ok(found)
    }
}

/// One worker thread for every lookup this process will ever make, alive for the process —
/// where the old machinery held up to 128 parked threads. Multi-thread flavour on purpose:
/// `block_on` against a `current_thread` runtime serializes callers, and a second host must
/// not queue behind a first one that is timing out.
static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();

fn runtime() -> Result<&'static tokio::runtime::Runtime, ClientError> {
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("cut-host resolve")
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| {
            ClientError::Transport(format!("name resolution has no runtime to run on ({e})"))
        })
}

/// `host`'s addresses over unicast DNS, by the system's own configuration, or a failure once
/// `deadline` passes. The timeout drops the lookup future — sockets closed, nothing left
/// running — which is the whole reason hickory is here instead of `to_socket_addrs`.
pub(crate) fn resolve_dns(
    host: &str,
    port: u16,
    deadline: Instant,
) -> Result<Vec<SocketAddr>, ClientError> {
    // Rebuilt per call: building parses resolv.conf (cheap), and a desktop runs for days
    // across VPN connects that rewrite it — a cached resolver keeps dead nameservers until
    // restart. The redial path runs once a second only while a host is unreachable.
    // ponytail: cache keyed on the config file's mtime, if this parse ever shows in a profile.
    let builder = Resolver::builder_tokio().map_err(|e| {
        ClientError::Transport(format!("could not read this machine's resolver configuration ({e})"))
    })?;
    lookup(
        // Deviation from the brief's literal listing: `ResolverBuilder::build` returns
        // `Result` in hickory-resolver 0.26.1 (confirmed against the vendored source), so
        // the brief's own pre-specified one-line fix applies here.
        builder.build().map_err(|e| ClientError::Transport(e.to_string()))?,
        host,
        port,
        deadline,
    )
}

/// Same lookup against a caller-supplied configuration. The seam exists so a test can point
/// resolution at a nameserver of its own choosing — a black hole — and hold the deadline to
/// its promise; the cfg makes "nothing outside tests reaches for it" the compiler's promise
/// rather than this comment's.
#[cfg(test)]
fn resolve_dns_with(
    config: ResolverConfig,
    host: &str,
    port: u16,
    deadline: Instant,
) -> Result<Vec<SocketAddr>, ClientError> {
    lookup(
        // Same version-sensitive fix as `resolve_dns`: `build()` is fallible here too.
        Resolver::builder_with_config(config, TokioRuntimeProvider::default())
            .build()
            .map_err(|e| ClientError::Transport(e.to_string()))?,
        host,
        port,
        deadline,
    )
}

fn lookup(
    resolver: hickory_resolver::TokioResolver,
    host: &str,
    port: u16,
    deadline: Instant,
) -> Result<Vec<SocketAddr>, ClientError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    // Spawned onto this module's own runtime and answered over a channel — never `block_on`:
    // the desktop dials from inside Tauri's tokio workers, and blocking a worker thread on a
    // runtime panics by design. The future carries its own timeout, so a caller that stops
    // listening abandons a lookup that tokio still cancels at the deadline.
    let (tx, rx) = std::sync::mpsc::channel();
    let owned = host.to_string();
    runtime()?.spawn(async move {
        let _ = tx.send(tokio::time::timeout(remaining, resolver.lookup_ip(owned.as_str())).await);
    });
    // The channel waits a beat longer than tokio's own timeout: the timeout inside the future
    // is the real clock, and the margin keeps a busy runtime from converting "answered right
    // at the wire" into a spurious refusal.
    match rx.recv_timeout(remaining + Duration::from_millis(250)) {
        Ok(Ok(Ok(found))) => Ok(found.iter().map(|ip| SocketAddr::new(ip, port)).collect()),
        Ok(Ok(Err(e))) => Err(ClientError::Transport(format!("could not resolve `{host}`: {e}"))),
        Ok(Err(_)) | Err(_) => {
            Err(ClientError::Transport(format!("`{host}` could not be resolved in time")))
        }
    }
}

/// `addr`'s socket addresses by whichever resolver its shape calls for, ordered for the
/// connect loop, or a failure once `deadline` passes. The one entry point `client.rs` dials
/// through; the contract — never block past the deadline — is what #126 bought.
pub(crate) fn resolve_by_deadline(
    addr: &str,
    deadline: Instant,
) -> Result<Vec<SocketAddr>, ClientError> {
    match route(addr)? {
        Route::Literal(literal) => Ok(vec![literal]),
        Route::Mdns { host, port } => Ok(order_for_connect(resolve_mdns(&host, port, deadline)?)),
        Route::Dns { host, port } => Ok(order_for_connect(resolve_dns(&host, port, deadline)?)),
    }
}

/// v4 before v6 — see the ordering test for why. Stable, so a responder's own order
/// survives within each family.
fn order_for_connect(mut addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
    addrs.sort_by_key(|a| a.is_ipv6());
    addrs
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_resolver::config::{NameServerConfig, ResolverConfig};

    /// A literal address must route without a lookup, whatever this machine's name
    /// resolution is doing — a host paired by IP stays reachable. Both families, since the
    /// bracket form is the one a v6 operator would type.
    #[test]
    fn literal_addresses_route_to_no_resolver() {
        assert_eq!(
            route("192.168.1.50:7878").unwrap(),
            Route::Literal("192.168.1.50:7878".parse().unwrap())
        );
        assert_eq!(
            route("[fe80::1]:7878").unwrap(),
            Route::Literal("[fe80::1]:7878".parse().unwrap())
        );
    }

    /// `.local` is mDNS whatever the casing, and however many trailing dots the operator's
    /// muscle memory added — DNS names are case-insensitive and the root dot is implicit.
    #[test]
    fn dot_local_routes_to_mdns_case_insensitively() {
        for spelled in ["cuthulhu-pi.local:7878", "Cuthulhu-Pi.LOCAL:7878", "cuthulhu-pi.local.:7878"] {
            assert_eq!(
                route(spelled).unwrap(),
                Route::Mdns { host: "cuthulhu-pi.local".into(), port: 7878 },
                "{spelled} should be mDNS"
            );
        }
    }

    /// Everything that is neither literal nor `.local` is unicast DNS: router names,
    /// hosts-file names, search-domain names. Single-label names land here too — that is
    /// the divergence the spec accepts (they no longer reach mDNS).
    #[test]
    fn other_names_route_to_dns() {
        assert_eq!(route("pi.lan:7878").unwrap(), Route::Dns { host: "pi.lan".into(), port: 7878 });
        assert_eq!(
            route("localhost:7878").unwrap(),
            Route::Dns { host: "localhost".into(), port: 7878 }
        );
        assert_eq!(
            route("cuthulhu-pi:7878").unwrap(),
            Route::Dns { host: "cuthulhu-pi".into(), port: 7878 }
        );
    }

    /// An address without a usable port cannot be dialled, and the error must say what the
    /// caller typed rather than what the parser choked on.
    #[test]
    fn an_address_without_a_port_is_refused() {
        for broken in ["cuthulhu-pi.local", "cuthulhu-pi.local:", "cuthulhu-pi.local:port", ":7878"] {
            let err = route(broken).expect_err(broken).to_string();
            assert!(err.contains(broken), "error should name `{broken}`: {err}");
        }
    }

    use std::time::{Duration, Instant};

    /// The hosts file is part of "unicast DNS" as an operator understands it — `localhost`
    /// and hand-pinned names must keep resolving now that `getaddrinfo` is out of the loop.
    #[test]
    fn dns_resolution_still_reads_the_hosts_file() {
        let addrs = resolve_dns("localhost", 7878, Instant::now() + Duration::from_secs(5))
            .expect("localhost must resolve");
        assert!(
            addrs.iter().any(|a| a.ip().is_loopback()),
            "localhost should include a loopback address: {addrs:?}"
        );
    }

    /// The property #126 exists to buy: a nameserver that never answers costs the caller its
    /// deadline and nothing more. The deaf nameserver is a socket this test binds and never
    /// reads: queries land in its buffer and no reply ever comes — silence that no network
    /// topology can spoil. (The conventional black-holed 10.255.255.1 turned out to answer
    /// fast on some networks, via ICMP, which converts the timeout under test into an
    /// ordinary error.) Built from parts — not `ResolverConfig::default()`, which carries
    /// real public nameservers that would answer and pass this vacuously.
    #[test]
    fn a_nameserver_that_never_answers_costs_only_the_deadline() {
        let deaf = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind the deaf nameserver");
        let mut ns = NameServerConfig::udp(deaf.local_addr().unwrap().ip());
        ns.connections[0].port = deaf.local_addr().unwrap().port();
        let silent = ResolverConfig::from_parts(None, vec![], vec![ns]);

        let start = Instant::now();
        let err = resolve_dns_with(
            silent,
            "cuthulhu-blackhole-test.example",
            7878,
            start + Duration::from_secs(1),
        )
        .expect_err("a nameserver that never answers must not produce one");
        // `deaf` lives to here, so the port cannot be reassigned to something that answers.
        drop(deaf);
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "gave up after {:?}, not by the deadline: {err}",
            start.elapsed()
        );
        assert!(err.to_string().contains("in time"), "should read as a timeout: {err}");
    }

    /// The bound is the property, not the answer: a name nobody on the network owns must
    /// come back by the deadline whether this runner's multicast works (query sent, nothing
    /// answers) or is blocked entirely (the daemon errors). Both are bounded outcomes; the
    /// unbounded one this test exists to forbid is waiting on an OS resolver.
    #[test]
    fn an_unanswered_local_name_costs_only_the_deadline() {
        // A name with this process's id in it, so no cached answer from a parallel test run
        // or a previous one can satisfy it.
        let host = format!("cuthulhu-nobody-{}.local", std::process::id());
        let start = Instant::now();
        let err = resolve_mdns(&host, 7878, start + Duration::from_secs(2))
            .expect_err("nobody on the network owns this name");
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "gave up after {:?}, not by the deadline: {err}",
            start.elapsed()
        );
    }

    /// The positive path needs multicast to actually loop back, which CI runners routinely
    /// refuse — so this is `#[ignore]`d and run by hand, and the real-hardware version lives
    /// in `apps/desktop/MANUAL-CHECKLIST.md`. Register a hostname on the shared daemon, then
    /// resolve it: mdns-sd answers its own queries over the wire, not via a shortcut.
    #[test]
    #[ignore = "needs working multicast loopback; run by hand: cargo test -p cut-host -- --ignored"]
    fn a_registered_local_name_resolves_over_multicast() {
        let daemon = daemon().expect("multicast daemon");
        let service = mdns_sd::ServiceInfo::new(
            "_cuthulhu-test._udp.local.",
            "resolver-plan-test",
            "cuthulhu-plan-test.local.",
            "127.0.0.1",
            7878,
            &[("spec", "2026-08-11")][..],
        )
        .expect("service info");
        daemon.register(service).expect("register");

        let addrs = resolve_mdns("cuthulhu-plan-test.local", 7878, Instant::now() + Duration::from_secs(5))
            .expect("a name this very daemon announces must resolve");
        assert!(
            addrs.iter().any(|a| a.ip().is_loopback()),
            "expected the registered loopback address: {addrs:?}"
        );
    }

    /// mDNS commonly answers with an IPv6 link-local first, and on a network where IPv6 is
    /// dead that is the one address that cannot work. Ordering v4 first removes that failure
    /// mode; the old code only budgeted around it. Stable sort, so within a family the
    /// responder's order survives.
    #[test]
    fn addresses_are_ordered_v4_before_v6_for_the_connect_loop() {
        let mixed: Vec<SocketAddr> = vec![
            "[fe80::1]:7878".parse().unwrap(),
            "192.168.1.50:7878".parse().unwrap(),
            "[fe80::2]:7878".parse().unwrap(),
            "192.168.1.51:7878".parse().unwrap(),
        ];
        let ordered = order_for_connect(mixed);
        assert_eq!(
            ordered,
            vec![
                "192.168.1.50:7878".parse::<SocketAddr>().unwrap(),
                "192.168.1.51:7878".parse().unwrap(),
                "[fe80::1]:7878".parse().unwrap(),
                "[fe80::2]:7878".parse().unwrap(),
            ]
        );
    }

    /// The literal fast path, end to end through the public entry point: a deadline already
    /// in the past, which every resolving route would fail on — answering anyway is only
    /// possible by not resolving. (Moved from `client.rs`, minus its assertion about the
    /// `RESOLVING` dedup set, which no longer exists to assert on.)
    #[test]
    fn an_address_that_is_already_an_address_is_not_resolved_at_all() {
        let resolved = resolve_by_deadline("192.168.1.50:7878", Instant::now())
            .expect("a literal address must not depend on any resolver");
        assert_eq!(resolved, vec!["192.168.1.50:7878".parse::<std::net::SocketAddr>().unwrap()]);
    }

    /// The desktop dials from inside Tauri's tokio workers, where blocking on a runtime
    /// panics by design ("cannot block the current thread from within a runtime") — a context
    /// the other tests never enter, which is how a `block_on` here first shipped as a panic
    /// on every unicast-name dial. Reproduce the context: a foreign runtime drives an async
    /// task that calls the sync resolver.
    #[test]
    fn dns_resolution_works_from_inside_a_foreign_tokio_runtime() {
        let foreign = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("test runtime");
        let addrs = foreign
            .block_on(async {
                resolve_dns("localhost", 7878, Instant::now() + Duration::from_secs(5))
            })
            .expect("resolving from inside a runtime must not panic");
        assert!(
            addrs.iter().any(|a| a.ip().is_loopback()),
            "localhost should include a loopback address: {addrs:?}"
        );
    }

    /// Two callers after the same name at once: mdns-sd keeps one listener per hostname, so
    /// unserialized concurrent queries tear each other's subscription down — the second
    /// `resolve_hostname` overwrites the first caller's channel, and either caller's stop
    /// kills the other's query. Serialized, both come back by their own deadlines; the
    /// second spends part of its budget waiting its turn.
    #[test]
    fn concurrent_resolves_of_the_same_name_are_both_answered_by_their_deadlines() {
        let host = format!("cuthulhu-shared-{}.local", std::process::id());
        let start = Instant::now();
        std::thread::scope(|s| {
            let workers: Vec<_> = (0..2)
                .map(|_| {
                    let host = host.clone();
                    s.spawn(move || {
                        resolve_mdns(&host, 7878, Instant::now() + Duration::from_secs(2))
                    })
                })
                .collect();
            for worker in workers {
                worker.join().expect("no panic").expect_err("nobody owns this name");
            }
        });
        assert!(
            start.elapsed() < Duration::from_secs(6),
            "each caller must be bounded by its own deadline, took {:?}",
            start.elapsed()
        );
    }
}
