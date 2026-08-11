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

Create the directory before anything writes into it, owned by the account created above:

```sh
sudo install -d -m 0700 -o cuthulhu -g cuthulhu /etc/cuthulhu /var/lib/cuthulhu
```

`0700` owned by `cuthulhu` rather than root: the unit below runs the daemon as `User=cuthulhu`, and
it has to be able to traverse the directory and read the file to load its own config.

`/etc/cuthulhu/cutd.toml`:

```toml
# The address to listen on — the Pi's own address on your network, so only
# devices on that network can reach it. Find yours with `hostname -I` if you
# don't already know it. See "Binding to every interface" below if you
# actually want this reachable from outside your LAN.
bind = "192.168.1.50:7878"

# One token per client, named so you can tell them apart. Generate each with:
#   head -c 32 /dev/urandom | base64
# Revoking a client is deleting its line and restarting; the others keep working.
[tokens]
workshop-laptop = "REPLACE-ME"
```

```sh
sudo chown cuthulhu:cuthulhu /etc/cuthulhu/cutd.toml
sudo chmod 0600 /etc/cuthulhu/cutd.toml
```

The certificate is generated into `/var/lib/cuthulhu/` on first run. Its fingerprint is printed at
startup — that is what you confirm when pairing a desktop.

## Serial cutters

A Puma is reached over serial. Give the daemon's user access to the port:

```sh
sudo usermod -a -G dialout cuthulhu
```

## USB cutters

A Cameo is reached directly over USB (`nusb` opens `/dev/bus/usb/...` on Linux), and Raspberry Pi
OS leaves those device nodes `root:root 0664`. The `uaccess` tag that would normally grant access
only applies to a logged-in seat user, which the `cuthulhu` system account is not, so without a
rule the daemon's claim fails permission-denied and it reports "no cutter is attached" even though
one is plugged in. Add a rule granting the `cuthulhu` group access to the Cameo's vendor id:

```
# /etc/udev/rules.d/60-cuthulhu.rules
SUBSYSTEM=="usb", ATTR{idVendor}=="3844", MODE="0660", GROUP="cuthulhu"
```

```sh
sudo udevadm control --reload-rules && sudo udevadm trigger
```

A Cameo already plugged in when the rule is added keeps its old permissions until it is replugged
(or the Pi is rebooted) — that is when udev re-evaluates the rule against it.

## Run it as a service

`/etc/systemd/system/cuthulhu-cutd.service`:

```ini
[Unit]
Description=Cuthulhu Cut Host
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=/usr/local/bin/cuthulhu-cutd --config /etc/cuthulhu/cutd.toml
Restart=on-failure
RestartSec=5
User=cuthulhu
StateDirectory=cuthulhu

[Install]
WantedBy=multi-user.target
```

`After=` alone is a documented no-op: nothing pulls `network-online.target` into the boot
transaction, so the ordering never applies without the matching `Wants=`. And a Pi commonly
reaches `multi-user.target` before DHCP hands it an address, so `bind`ing a specific address like
the example above fails on the first try; `RestartSec=5` gives the network time to come up instead
of exhausting systemd's default restart burst and leaving the unit `failed`.

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

## Rotating a token

Change that client's line under `[tokens]` in `cutd.toml` and restart. Only that desktop needs to be
paired again — the other entries, and the clients holding them, are untouched.

## Revoking one client

Delete that client's line from `[tokens]` and restart:

```sh
sudo systemctl restart cuthulhu-cutd
```

Every other client keeps working. `journalctl -u cuthulhu-cutd` names which client authenticated
and which dispatched each Job, so you can tell which line to remove.
