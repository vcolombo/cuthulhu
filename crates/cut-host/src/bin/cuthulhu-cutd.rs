// SPDX-License-Identifier: GPL-3.0-or-later
//! The Cut Host daemon: owns the Transports to every attached cutter and runs Jobs
//! on them for authenticated clients on the local network.

use std::path::PathBuf;
use std::sync::Arc;

use cut_host::config::Config;
use cut_host::host::Host;
use cut_host::serve::serve;
use driver_registry::HardwareBackendFactory;

fn main() {
    let mut args = std::env::args().skip(1);
    let mut config_path = PathBuf::from("/etc/cuthulhu/cutd.toml");
    let mut allow_public_bind = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => match args.next() {
                Some(p) => config_path = PathBuf::from(p),
                None => fail("--config needs a path"),
            },
            "--allow-public-bind" => allow_public_bind = true,
            other => fail(&format!("unknown argument `{other}`")),
        }
    }

    let config = match Config::load(&config_path) {
        Ok(c) => c,
        Err(e) => fail(&format!("{}: {e}", config_path.display())),
    };

    let host = Host::start(Arc::new(HardwareBackendFactory));
    for device in host.devices() {
        eprintln!("cut host: {} is a {}", device.instance_id, device.machine_id);
    }
    if host.devices().is_empty() {
        eprintln!("cut host: no cutter is attached; clients will see an empty list");
    }

    if let Err(e) = serve(host, config, allow_public_bind) {
        fail(&e.to_string());
    }
}

fn fail(message: &str) -> ! {
    eprintln!("cuthulhu-cutd: {message}");
    std::process::exit(1);
}
