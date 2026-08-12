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

#[cfg(test)]
mod tests {
    use super::*;

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
}
