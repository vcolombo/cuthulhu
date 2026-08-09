<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Running a Cut Host on a Raspberry Pi

A Cut Host owns the USB and serial connections to your cutters and runs Jobs on them for clients on
your network. It owns the cut: once a Job starts, closing the laptop that sent it does not stop it.

## Build

Build on the Pi, or cross-compile:

```sh
cross build --release --target aarch64-unknown-linux-gnu -p cut-host --bin cuthulhu-cutd
```

Copy the binary to `/usr/local/bin/cuthulhu-cutd`.

## Configure

`/etc/cuthulhu/cutd.toml`:

```toml
# The address to listen on. `0.0.0.0` is not a private address as far as the
# daemon is concerned, so binding it — as below — needs `--allow-public-bind` on
# the command line (see the systemd unit) or the daemon refuses to start.
bind = "0.0.0.0:7878"

# The shared secret a client presents. Generate one and keep it secret:
#   head -c 32 /dev/urandom | base64
token = "REPLACE-ME"
```

```sh
sudo install -d -m 0700 /etc/cuthulhu /var/lib/cuthulhu
sudo chmod 0600 /etc/cuthulhu/cutd.toml
```

The certificate is generated into `/var/lib/cuthulhu/` on first run. Its fingerprint is printed at
startup — that is what you confirm when pairing a desktop.

## Serial cutters

A Puma is reached over serial. Give the daemon's user access to the port:

```sh
sudo usermod -a -G dialout cuthulhu
```

## Run it as a service

`/etc/systemd/system/cuthulhu-cutd.service`:

```ini
[Unit]
Description=Cuthulhu Cut Host
After=network-online.target

[Service]
ExecStart=/usr/local/bin/cuthulhu-cutd --config /etc/cuthulhu/cutd.toml --allow-public-bind
Restart=on-failure
User=cuthulhu
StateDirectory=cuthulhu

[Install]
WantedBy=multi-user.target
```

```sh
sudo systemctl enable --now cuthulhu-cutd
journalctl -u cuthulhu-cutd -f
```

A restart kills any cut in flight. `systemctl restart` while a Job is running will ruin the material
on the mat — check `journalctl` first.

## Reaching it

Raspberry Pi OS advertises its hostname over mDNS, so the host is at `<hostname>.local:7878`. No
discovery service is involved and none is needed.

## Rotating the token

Change `token` in `cutd.toml` and restart. Every paired desktop must be paired again — the token is
the whole of the trust, and there is one for the host.
