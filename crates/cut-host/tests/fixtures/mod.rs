// SPDX-License-Identifier: GPL-3.0-or-later
//! A Cut Host on a loopback port, with two mock cutters behind it.

use std::net::TcpListener;
use std::sync::Arc;

use cut_host::config::Config;
use cut_host::host::Host;
use cut_host::serve::{fingerprint_of_cert_dir, serve_on};
use driver_core::manager::CutPass;
use driver_core::{Job, Settings};
use geometry::Point;

pub const TOKEN: &str = "test-token";
pub const CAMEO: &str = "usb:1:4";

pub struct TestHost {
    pub addr: String,
    pub fingerprint: String,
    /// Held so the certificate outlives the host. Dropping it deletes the
    /// directory `serve_on` reads its key from.
    _dir: tempfile::TempDir,
}

/// Binds port 0 so tests can run concurrently, then reads back the port the OS
/// chose. `serve_on` takes an already-bound listener for exactly this reason.
pub fn start_test_host() -> TestHost {
    let dir = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    let config = Config {
        bind: listener.local_addr().unwrap(),
        tokens: [("test-client".to_string(), TOKEN.to_string())].into_iter().collect(),
        max_frame: cut_host::frame::DEFAULT_MAX_FRAME,
        cert_dir: dir.path().to_path_buf(),
    };
    // Generates the certificate if it is not there, so the client has something to
    // pin before the server has accepted anything.
    let fingerprint = fingerprint_of_cert_dir(&config.cert_dir).unwrap();
    let host = Host::start(Arc::new(cut_host::host::testing::TwoCutterFactory));

    std::thread::spawn(move || {
        let _ = serve_on(listener, host, config);
    });
    TestHost { addr, fingerprint, _dir: dir }
}

pub fn square_pass() -> CutPass {
    CutPass {
        job: Job {
            polylines: vec![vec![
                Point { x: 0.0, y: 0.0 }, Point { x: 10.0, y: 0.0 },
                Point { x: 10.0, y: 10.0 }, Point { x: 0.0, y: 0.0 },
            ]],
            settings: Settings::default(),
        },
    }
}
