// SPDX-License-Identifier: GPL-3.0-or-later
//! A Cut Host: a machine that owns Transports to one or more cutters and runs Jobs
//! on them for remote clients.
//!
//! The host owns the cut. A client may detach mid-Job — its laptop closing, its
//! network dropping — and the Job continues, which is the whole reason this crate
//! exists rather than a Transport that forwards bytes to a desktop still driving.

pub mod check;
// The desktop's half of the crate. Feature-gated so the Pi daemon builds without the
// resolver stack it cannot call — see Cargo.toml's `client` feature.
#[cfg(feature = "client")]
pub mod client;
pub mod config;
pub mod frame;
pub mod host;
pub mod protocol;
pub mod serve;
pub mod shutdown;
