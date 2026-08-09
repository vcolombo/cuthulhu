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

## Create the daemon's user

`cuthulhu-cutd` runs as an unprivileged system account, never as root:

```sh
sudo useradd --system --no-create-home --shell /usr/sbin/nologin cuthulhu
```

Everything below that touches `/etc/cuthulhu` or the `cuthulhu` group assumes this account already
exists.

## Configure

`/etc/cuthulhu/cutd.toml`:

```toml
# The address to listen on — the Pi's own address on your network, so only
# devices on that network can reach it. Find yours with `hostname -I` if you
# don't already know it. See "Binding to every interface" below if you
# actually want this reachable from outside your LAN.
bind = "192.168.1.50:7878"

# The shared secret a client presents. Generate one and keep it secret:
#   head -c 32 /dev/urandom | base64
token = "REPLACE-ME"
```

```sh
sudo install -d -m 0700 -o cuthulhu -g cuthulhu /etc/cuthulhu /var/lib/cuthulhu
sudo chown cuthulhu:cuthulhu /etc/cuthulhu/cutd.toml
sudo chmod 0600 /etc/cuthulhu/cutd.toml
```

`0700` owned by `cuthulhu` rather than root: the unit below runs the daemon as `User=cuthulhu`, and
it has to be able to traverse the directory and read the file to load its own config.

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
ExecStart=/usr/local/bin/cuthulhu-cutd --config /etc/cuthulhu/cutd.toml
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

## Binding to every interface

`0.0.0.0` is not a private address as far as the daemon is concerned — it means "every interface,"
which includes ones you did not mean to expose (a second NIC, a VPN tunnel, Docker's bridge). The
daemon refuses that bind unless `--allow-public-bind` is added to the unit's `ExecStart`, because
the thing on the other end of an open port here can be told to move a blade, and that should be a
decision you made on purpose rather than a default you inherited. If you genuinely need it — the Pi
has multiple LAN interfaces and you want the daemon reachable on all of them, say — set
`bind = "0.0.0.0:7878"` in `cutd.toml` and add the flag. Anything that can reach that port is then
trusted with the same authority as a paired desktop, limited only by the token.

## Rotating the token

Change `token` in `cutd.toml` and restart. Every paired desktop must be paired again — the token is
the whole of the trust, and there is one for the host.
