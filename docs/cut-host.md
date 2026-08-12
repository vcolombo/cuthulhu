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

`cross` needs Docker running, and takes the rest from the repository: `Cross.toml` installs the
target's `libudev` into the build image, and `.cargo/config.toml` tells `pkg-config` to look there.
Both are needed because `serialport` links a C library, and a cross build must find the *Pi's*
copy of it rather than the build machine's.

Not on an Apple Silicon Mac, though: `cross` 0.2.5 wants an `x86_64` Linux toolchain that `rustup`
refuses to install on an ARM host, and it stops before it reaches Docker at all. Build on the Pi,
cross-compile from an `x86_64` machine, or do it in a container:

```sh
docker run --rm --platform linux/amd64 -v "$PWD":/work -w /work \
  -e CARGO_TARGET_DIR=/work/target/cross \
  -e CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc \
  rust:1-bookworm bash -c '
    dpkg --add-architecture arm64 && apt-get update &&
    apt-get install -y gcc-aarch64-linux-gnu libc6-dev-arm64-cross libudev-dev:arm64 pkg-config &&
    rustup target add aarch64-unknown-linux-gnu &&
    cargo build --release --target aarch64-unknown-linux-gnu -p cut-host --bin cuthulhu-cutd'
```

The binary lands in `target/cross/aarch64-unknown-linux-gnu/release/cuthulhu-cutd`. Its own target
directory, because the container's build scripts are Linux binaries and would otherwise be written
over the macOS ones in `target/release/build`. `libc6-dev-arm64-cross` is the easy one to leave
out and the confusing one to debug: without it the aarch64 compiler is present but has no headers,
and the first C dependency fails on a missing `bits/libc-header-start.h`.

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
TimeoutStopSec=30min
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

`TimeoutStopSec` is the one line here that decides whether a cut survives a stop, and it must be
longer than your longest realistic Job — see "Stopping it while a cut is running" below.

```sh
sudo systemctl enable --now cuthulhu-cutd
journalctl -u cuthulhu-cutd -f
```

## Stopping it while a cut is running

`systemctl stop`, `systemctl restart`, a package upgrade and a plain `kill` all send SIGTERM, and
the daemon refuses to end a Job on one. It logs which cutter is still busy and waits, then exits by
itself the moment the last cut finishes — a stop issued mid-Job is deferred, not rejected, so you
can leave the command running and it completes when the material does.

**A Pass waiting for you counts as a cut in progress.** A Puma cannot be polled, so between
colours it parks and waits for somebody to swap the material — and the daemon treats that as
active, because the alternative is abandoning a Job somebody fully intends to finish. It will wait
there as long as the Pass does. Upgrade a package at 6pm with a Puma parked mid-Job and nobody in
the room, and `systemctl stop` sits for the whole of `TimeoutStopSec` on a machine that is not
cutting and will not resume until someone walks over to it. The log line names it —
`waiting for an operator` beside the cutter — and the second signal below is how you stop anyway.

That waiting is only as long as systemd allows. **`TimeoutStopSec` must exceed your longest
realistic cut**, including any time a Pass may sit waiting for a person. systemd sends SIGKILL when it expires, and SIGKILL cannot be refused by anything —
leave it at the default (90 seconds on most systems) and a stop during a half-hour Job abandons the
cut exactly as before, while the daemon's refusal in the log makes it look like it did not.
`TimeoutStopSec=infinity` waits forever, at the cost of a genuinely wedged daemon being able to
block shutdown and reboot indefinitely; the 30 minutes above is the compromise, and it is a number
to raise if your Jobs are longer.

To stop anyway, signal a second time — the first signal is the guard, the second is you saying you
mean it, and the cut is abandoned:

```sh
sudo systemctl kill --signal=SIGTERM cuthulhu-cutd
```

There is no flag for this, on purpose: the force has to be reachable by whoever is already holding
the signal rather than needing an edit to the unit file. Ctrl-C twice does the same when running the
daemon in a terminal.

Abandoning a cut leaves the blade wherever it stopped and the material unusable. Prefer cancelling
the Job from the desktop first — that stops the machine deliberately, after which the pending stop
completes on its own.

## Reaching it

Raspberry Pi OS advertises its hostname over mDNS, so the host is at `<hostname>.local:7878`. No
discovery service is involved and none is needed. Address a host as `name.local:port` or
`ip:port`, spelled out: a bare single-label name (`cuthulhu-pi:7878`) is looked up over unicast
DNS with the desktop's search domains, not over mDNS, so it only works where the router
registers DHCP names.

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
and which dispatched each Job, so you can tell which line to remove. A restart issued while a cut is
running waits for it, as above — the revocation takes effect when the Job ends.
