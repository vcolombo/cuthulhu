<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Cut Host phase 2, part A: the daemon's prerequisites — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `crates/cut-host` safe for a desktop to hold open and poll — a frame read that cannot block forever, and per-client tokens so revoking one desktop does not lock out the rest.

**Architecture:** Two changes, both confined to the daemon and its client library. `read_frame` waits as long as a peer likes for a frame to *begin* and bounds the time it may take to *finish*, so an idle connection stays idle and a stalled one fails. `cutd.toml`'s single `token` scalar becomes a `[tokens]` table keyed by a name the operator chooses, and the daemon logs which name authenticated.

**Tech Stack:** Rust workspace, `cargo test --workspace --locked`. No new dependencies.

Spec: `docs/superpowers/specs/2026-08-09-cut-host-desktop-design.md`. Issues: #97, and the token half of the spec's *Per-client named tokens* decision.

## Why this is its own plan

Phase 2's spec names two prerequisites. One — #100, stable device identity — is already fixed in PR #101. The other is #97, and the spec is explicit about why it stops being optional:

> Polling on a timer over a held connection turns a hung Pi into a frozen application; today it costs only a stuck test.

Both changes here are daemon-side, testable without a desktop, and touch no file the desktop half of phase 2 touches. They ship as their own PR so the desktop work starts on a foundation that cannot freeze.

**Nothing in `apps/` is touched by this plan.** `git diff main -- apps/` must stay empty for it.

## Global Constraints

- **SPDX header on every file**: `// SPDX-License-Identifier: GPL-3.0-or-later`, `# ` for TOML, `<!-- -->` for Markdown.
- **`cargo test --workspace --locked`** is what CI runs; `--locked` is mandatory. Neither task adds a dependency, so `Cargo.lock` must not change — if it does, something went wrong.
- **`CONTEXT.md` is normative vocabulary.** Use **Pass**, **Job**, **Driver**, **Transport**, **Preflight**, **Cut Host**. Never "proxy", "server", "relay" or "bridge" for the Cut Host.
- **Comments explain why, not what.** Where a step below carries a comment, that comment is part of the deliverable. Do not add restating ones.
- **`// ponytail:` marks a deliberate simplification** with its ceiling and upgrade path.
- **Commit subjects are imperative with the reason attached.** Keep the repo's `Co-Authored-By:` trailer. Prose — comments, docs, commit bodies — carries no process narration: no "as requested", no "per the plan", no agent names.
- The crate builds warning-free today. Verify with `cargo clean -p cut-host` then a rebuild before each commit.

---

### Task 1: A frame read that cannot block forever

**Files:**
- Modify: `crates/cut-host/src/frame.rs` — `FrameError`, `read_frame`, and a new deadline-aware read loop
- Modify: `crates/cut-host/src/serve.rs` — pass a body deadline at the two `read_frame` call sites
- Modify: `crates/cut-host/src/client.rs` — pass a body deadline at its `read_frame` call sites, and set a socket read timeout

**Interfaces:**
- Consumes: `FrameError`, `read_frame`, `DEFAULT_MAX_FRAME` as they stand.
- Produces:
  - `FrameError::Timeout` — a new variant.
  - `pub const DEFAULT_BODY_TIMEOUT: Duration = Duration::from_secs(30);`
  - `pub const SOCKET_POLL_INTERVAL: Duration = Duration::from_secs(1);`
  - `pub fn read_frame<R: Read, T: DeserializeOwned>(r: &mut R, max: usize, body_timeout: Duration) -> Result<T, FrameError>` — the signature gains a third parameter; every caller must pass one.

**The design, and why it is not just `set_read_timeout`.**

A socket timeout alone cannot express what is wanted, because the two waits are different:

- **Waiting for a frame to begin** must be unbounded. A desktop that polls once a second leaves the connection idle in between, and a daemon that timed that out would drop every client.
- **Waiting for a frame to finish** must be bounded. Once a header has arrived, the peer has promised a body; a peer that stops mid-body — a hung Pi, a Wi-Fi drop that never sends a RST — must fail rather than hold the reader forever.

So the deadline starts when the header lands, not when the read begins. That also closes a slowloris on the daemon: eight connections each sending a header and stalling would otherwise pin every worker, with `MAX_CLIENTS` making it eight connections' work to do it.

`read_exact` cannot be used for either half. On a socket with `SO_RCVTIMEO` set, a read that times out returns `WouldBlock` or `TimedOut`, and `read_exact` gives no way to know how much it consumed before failing — so a retry would corrupt the frame. The loop below tracks its own fill and retries only where retrying is safe.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/cut-host/src/frame.rs`:

```rust
    use std::io::Cursor;
    use std::time::Duration;

    /// A reader that yields its script and then reports `WouldBlock` forever, as a socket with
    /// `SO_RCVTIMEO` does when the peer has stopped talking without closing.
    ///
    /// The sleep matters: a real socket blocks for its timeout before reporting `WouldBlock`, and
    /// that is what paces `fill`'s retry loop. A fake that answered instantly would make the loop
    /// spin a core and would misrepresent what the code does in production.
    struct StallsAfter {
        given: Cursor<Vec<u8>>,
    }
    impl StallsAfter {
        fn new(bytes: Vec<u8>) -> StallsAfter {
            StallsAfter { given: Cursor::new(bytes) }
        }
    }
    impl Read for StallsAfter {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.given.read(buf)? {
                0 => {
                    std::thread::sleep(Duration::from_millis(20));
                    Err(io::Error::new(io::ErrorKind::WouldBlock, "no data yet"))
                }
                n => Ok(n),
            }
        }
    }

    /// A reader that is silent for a while and then delivers its frame — a connection that was
    /// idle between polls, which is the normal case and must not be a fault.
    struct QuietThenSpeaks {
        quiet_reads_left: usize,
        given: Cursor<Vec<u8>>,
    }
    impl Read for QuietThenSpeaks {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.quiet_reads_left > 0 {
                self.quiet_reads_left -= 1;
                std::thread::sleep(Duration::from_millis(20));
                return Err(io::Error::new(io::ErrorKind::WouldBlock, "no data yet"));
            }
            self.given.read(buf)
        }
    }

    fn framed(value: &str) -> Vec<u8> {
        let mut buf = Vec::new();
        write_frame(&mut buf, &value.to_string()).unwrap();
        buf
    }

    /// The failure this task exists for: a peer that sends a header and then stops must not hold
    /// the reader forever. Before this change the read blocked with no deadline at all.
    #[test]
    fn a_body_that_never_arrives_times_out_rather_than_blocking() {
        let mut header_only = Vec::new();
        header_only.extend_from_slice(&(64u32).to_be_bytes());

        let started = std::time::Instant::now();
        let result = read_frame::<_, String>(
            &mut StallsAfter::new(header_only),
            DEFAULT_MAX_FRAME,
            Duration::from_millis(200),
        );
        assert!(matches!(result, Err(FrameError::Timeout)), "got {result:?}");
        assert!(started.elapsed() < Duration::from_secs(5), "it waited far past its deadline");
    }

    /// The other half of the rule: waiting for a frame to *begin* is unbounded, because a client
    /// that polls once a second leaves the connection idle in between and must not be dropped.
    ///
    /// Asserted without leaking a blocked thread: the reader is silent for 200ms — four times the
    /// body timeout — and then speaks. A reader that started its deadline before the header would
    /// return `Timeout`; the correct one reads the frame.
    #[test]
    fn an_idle_connection_is_not_timed_out_before_a_frame_begins() {
        let mut idle_then_busy =
            QuietThenSpeaks { quiet_reads_left: 10, given: Cursor::new(framed("hello")) };
        let got: String =
            read_frame(&mut idle_then_busy, DEFAULT_MAX_FRAME, Duration::from_millis(50))
                .expect("an idle connection must not be dropped before a frame begins");
        assert_eq!(got, "hello");
    }

    /// A frame that arrives in pieces, with stalls between them, must still be read — the
    /// deadline bounds the whole body, not each read.
    #[test]
    fn a_body_arriving_in_pieces_is_reassembled() {
        let bytes = framed("hello");
        let mut piecewise = StallsAfter::new(bytes);
        let got: String =
            read_frame(&mut piecewise, DEFAULT_MAX_FRAME, Duration::from_secs(5)).unwrap();
        assert_eq!(got, "hello");
    }

    /// A peer that closes mid-body is a fault, not a clean end — `Eof` means "closed between
    /// frames" and a caller loops on it.
    #[test]
    fn a_peer_that_closes_mid_body_is_a_fault_not_an_eof() {
        let mut truncated = Vec::new();
        truncated.extend_from_slice(&(64u32).to_be_bytes());
        truncated.extend_from_slice(b"{\"partial\":");

        let result =
            read_frame::<_, String>(&mut Cursor::new(truncated), DEFAULT_MAX_FRAME, Duration::from_secs(5));
        assert!(matches!(result, Err(FrameError::Io(_))), "got {result:?}");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cut-host --locked frame`

Expected: FAIL to compile — `read_frame` takes 2 arguments but 3 were supplied, and `FrameError::Timeout` does not exist.

- [ ] **Step 3: Add the variant and the deadline-aware read**

In `crates/cut-host/src/frame.rs`, add `Timeout` to `FrameError`:

```rust
#[derive(Debug)]
pub enum FrameError {
    /// The peer closed cleanly between frames. Not a fault.
    Eof,
    /// A frame began and did not finish inside its deadline.
    Timeout,
    TooLarge { len: usize, max: usize },
    Io(String),
    Malformed(String),
}
```

and its `Display` arm, beside the others:

```rust
            FrameError::Timeout => write!(f, "a frame began and did not finish in time"),
```

Add the constants beside `DEFAULT_MAX_FRAME`:

```rust
/// How long a frame has to finish once its header has arrived. Generous: a large cut is
/// megabytes of JSON over a home network, and this is a fault deadline rather than a
/// performance target.
pub const DEFAULT_BODY_TIMEOUT: Duration = Duration::from_secs(30);

/// How often a blocked read wakes to re-check its deadline. The socket's own `SO_RCVTIMEO`,
/// set by whoever owns the connection; the value only decides how promptly a stalled frame
/// is noticed, not how long it is tolerated.
pub const SOCKET_POLL_INTERVAL: Duration = Duration::from_secs(1);
```

and `use std::time::{Duration, Instant};` to the imports.

Then add the read loop above `read_frame`:

```rust
/// Fill `buf` completely, or fail — retrying reads that merely found no data yet, and giving up
/// when `deadline` passes.
///
/// `read_exact` cannot be used here. On a socket carrying `SO_RCVTIMEO` a quiet moment surfaces
/// as `WouldBlock`, and `read_exact` does not say how much it consumed before failing, so a retry
/// would resume mid-frame and corrupt it. This tracks its own fill so a retry is safe.
///
/// `deadline` of `None` waits forever, which is what waiting for a frame to *begin* must do: a
/// client that polls once a second is idle in between and must not be dropped for it.
///
/// The retry is paced by the socket, not by this loop: callers set `SO_RCVTIMEO`
/// (`SOCKET_POLL_INTERVAL`), so a quiet read blocks for that long before returning `WouldBlock`.
/// On a reader with no timeout at all this would spin, which is why both call sites set one.
fn fill(r: &mut impl Read, buf: &mut [u8], deadline: Option<Instant>) -> Result<(), FrameError> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => {
                return Err(if filled == 0 {
                    FrameError::Eof
                } else {
                    // Not `Eof`: a caller loops on `Eof` meaning "the peer left between frames",
                    // and a peer that vanished mid-frame left something behind.
                    FrameError::Io("the peer closed part-way through a frame".into())
                })
            }
            Ok(n) => filled += n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
                ) =>
            {
                if deadline.is_some_and(|d| Instant::now() >= d) {
                    return Err(FrameError::Timeout);
                }
            }
            Err(e) => return Err(FrameError::Io(e.to_string())),
        }
    }
    Ok(())
}
```

Then replace `read_frame`'s body:

```rust
/// Read one frame. Waits indefinitely for it to begin and gives it `body_timeout` to finish.
///
/// The two waits differ on purpose. An idle connection is normal — a desktop polling once a
/// second leaves one idle in between — so a header that has not arrived is not a fault. A body
/// that has not arrived is: the peer promised it in the header, and a peer that stops mid-frame
/// would otherwise hold this reader, and whatever lock is above it, forever.
pub fn read_frame<R: Read, T: DeserializeOwned>(
    r: &mut R,
    max: usize,
    body_timeout: Duration,
) -> Result<T, FrameError> {
    let mut header = [0u8; 4];
    fill(r, &mut header, None)?;

    let len = u32::from_be_bytes(header) as usize;
    // Before the allocation, not after: the whole point of the cap.
    if len > max {
        return Err(FrameError::TooLarge { len, max });
    }
    let mut body = vec![0u8; len];
    fill(r, &mut body, Some(Instant::now() + body_timeout))?;
    serde_json::from_slice(&body).map_err(|e| FrameError::Malformed(e.to_string()))
}
```

- [ ] **Step 4: Update the existing frame tests for the new signature**

Every existing call in `mod tests` gains a third argument. Use `DEFAULT_BODY_TIMEOUT` for all of them — none is testing the deadline:

```rust
        let first: String = read_frame(&mut cursor, DEFAULT_MAX_FRAME, DEFAULT_BODY_TIMEOUT).unwrap();
```

Do not change any assertion. If one now fails, the change altered behaviour it should not have — say so rather than editing the expectation.

One existing test needs a closer look: `a_truncated_body_is_malformed_not_a_hang` accepts `Io(_) | Malformed(_)`. It still passes, and `a_peer_that_closes_mid_body_is_a_fault_not_an_eof` from Step 1 now pins the `Io` case exactly, so leave the old one as it is.

- [ ] **Step 5: Pass a deadline from both callers, and set the socket timeout**

In `crates/cut-host/src/serve.rs`, the token read and the request read each gain the timeout:

```rust
    let presented: String = read_frame(&mut tls_stream, 1024, DEFAULT_BODY_TIMEOUT).map_err(io::Error::other)?;
```

```rust
        match read_frame::<_, Request>(&mut tls_stream, max_frame, DEFAULT_BODY_TIMEOUT) {
```

Import `DEFAULT_BODY_TIMEOUT` alongside the existing `frame` imports.

Then give the accepted socket a read timeout in `serve_client`, immediately after `peer` is read and before the TLS session is built:

```rust
    // The frame layer re-checks its deadline whenever a read comes back empty, so the socket
    // needs to come back empty rather than block indefinitely. This value only sets how promptly
    // a stalled frame is noticed; `DEFAULT_BODY_TIMEOUT` decides how long one is tolerated.
    stream
        .set_read_timeout(Some(SOCKET_POLL_INTERVAL))
        .map_err(|e| io::Error::other(format!("could not set a read timeout: {e}")))?;
```

In `crates/cut-host/src/client.rs`, do the same for the client's own socket in `connect`, after `TcpStream::connect` and before the TLS session is built:

```rust
        tcp.set_read_timeout(Some(crate::frame::SOCKET_POLL_INTERVAL))
            .map_err(|e| ClientError::Transport(e.to_string()))?;
```

and pass `crate::frame::DEFAULT_BODY_TIMEOUT` at each `read_frame` call in that file.

- [ ] **Step 6: Run the whole suite**

Run: `cargo test --workspace --locked`

Expected: PASS. Run `cargo test -p cut-host --locked --test end_to_end` three times as well — this changes the socket behaviour under every existing network test, and a regression here would show as intermittency rather than a clean failure.

- [ ] **Step 7: Commit**

```bash
git add crates/cut-host/src/frame.rs crates/cut-host/src/serve.rs crates/cut-host/src/client.rs
git commit -m "Give a frame a deadline once it has begun, so a peer that stops cannot hold the reader"
```

---

### Task 2: Per-client tokens, so revoking one desktop does not lock out the rest

**Files:**
- Modify: `crates/cut-host/src/config.rs` — `[tokens]` table, and refusing the old scalar by name
- Modify: `crates/cut-host/src/serve.rs` — `token_matches` returns which name matched; the daemon logs it
- Modify: `docs/cut-host.md` — the config example and a section on revoking one client

**Interfaces:**
- Consumes: `Config`, `ConfigError`, `token_matches` as they stand.
- Produces:
  - `Config::tokens: BTreeMap<String, String>` replaces `Config::token: String`
  - `ConfigError::LegacyToken` — a new variant
  - `pub fn match_token(presented: &str, tokens: &BTreeMap<String, String>) -> Option<String>` replaces `token_matches`, returning the *name* that matched

**Why the old form is refused rather than accepted.** A daemon that quietly kept working with a single `token = "…"` would leave an operator believing they had per-client revocation when they had one shared key — the exact belief OctoPrint's docs warn against for its own global key. Nothing is deployed yet, so refusing costs a line in the install guide and no migration.

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `crates/cut-host/src/config.rs`:

```rust
    #[test]
    fn a_config_reads_a_table_of_named_tokens() {
        let dir = write_config(
            "bind = \"127.0.0.1:7878\"\n\n[tokens]\nworkshop-laptop = \"aaa\"\noffice-desktop = \"bbb\"\n",
        );
        let config = Config::load(&dir.path().join("cutd.toml")).unwrap();
        assert_eq!(config.tokens.get("workshop-laptop").map(String::as_str), Some("aaa"));
        assert_eq!(config.tokens.get("office-desktop").map(String::as_str), Some("bbb"));
    }

    /// Refused rather than accepted as an unnamed token: a daemon that kept working would leave
    /// an operator believing they had per-client revocation when they had one shared key.
    #[test]
    fn the_old_single_token_form_is_refused_by_name() {
        let dir = write_config("bind = \"127.0.0.1:7878\"\ntoken = \"s3cret\"\n");
        match Config::load(&dir.path().join("cutd.toml")) {
            Err(ConfigError::LegacyToken) => {}
            other => panic!("expected LegacyToken, got {other:?}"),
        }
    }

    #[test]
    fn the_refusal_names_the_form_to_use_instead() {
        let message = ConfigError::LegacyToken.to_string();
        assert!(message.contains("[tokens]"), "the message must name the replacement: {message}");
    }

    #[test]
    fn a_config_with_no_tokens_at_all_is_refused() {
        let dir = write_config("bind = \"127.0.0.1:7878\"\n");
        assert!(matches!(Config::load(&dir.path().join("cutd.toml")), Err(ConfigError::NoToken)));

        let empty = write_config("bind = \"127.0.0.1:7878\"\n\n[tokens]\n");
        assert!(matches!(Config::load(&empty.path().join("cutd.toml")), Err(ConfigError::NoToken)));
    }

    /// An empty value would authorize everyone that guessed an empty string.
    #[test]
    fn a_token_with_an_empty_value_is_refused() {
        let dir = write_config("bind = \"127.0.0.1:7878\"\n\n[tokens]\nlaptop = \"\"\n");
        assert!(matches!(Config::load(&dir.path().join("cutd.toml")), Err(ConfigError::NoToken)));
    }
```

And to `mod tests` in `crates/cut-host/src/serve.rs`:

```rust
    fn tokens() -> std::collections::BTreeMap<String, String> {
        [("workshop-laptop".to_string(), "aaa".to_string()),
         ("office-desktop".to_string(), "bbb".to_string())]
            .into_iter()
            .collect()
    }

    #[test]
    fn a_token_matches_only_itself_and_reports_which_name_it_was() {
        assert_eq!(match_token("aaa", &tokens()).as_deref(), Some("workshop-laptop"));
        assert_eq!(match_token("bbb", &tokens()).as_deref(), Some("office-desktop"));
        assert_eq!(match_token("aab", &tokens()), None);
        assert_eq!(match_token("aa", &tokens()), None, "a prefix is not a match");
        assert_eq!(match_token("", &tokens()), None);
    }

    /// Revoking one client must leave the others working — the property the whole change exists
    /// for, and the one a shared key cannot offer.
    #[test]
    fn removing_one_token_leaves_the_others_working() {
        let mut remaining = tokens();
        remaining.remove("workshop-laptop");
        assert_eq!(match_token("aaa", &remaining), None, "the revoked client is out");
        assert_eq!(match_token("bbb", &remaining).as_deref(), Some("office-desktop"));
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cut-host --locked config` then `cargo test -p cut-host --locked serve`

Expected: FAIL to compile — `Config` has no field `tokens`, `ConfigError` has no variant `LegacyToken`, and `match_token` does not exist.

- [ ] **Step 3: Change the config**

In `crates/cut-host/src/config.rs`, add `use std::collections::BTreeMap;` and change `ConfigFile` and `Config`:

```rust
#[derive(Deserialize)]
struct ConfigFile {
    bind: Option<String>,
    /// Only read so the old single-token form can be refused by name rather than ignored.
    token: Option<String>,
    tokens: Option<BTreeMap<String, String>>,
    max_frame: Option<usize>,
    cert_dir: Option<PathBuf>,
}

pub struct Config {
    pub bind: SocketAddr,
    /// Named per client, so revoking one desktop leaves the others working. A `BTreeMap` rather
    /// than a `HashMap` so the daemon's startup log lists them in a stable order.
    pub tokens: BTreeMap<String, String>,
    pub max_frame: usize,
    pub cert_dir: PathBuf,
}
```

Add the error variant and its message:

```rust
    /// The pre-`[tokens]` form. Refused rather than read as an unnamed token: a daemon that
    /// kept working would leave an operator believing they had per-client revocation when they
    /// had one shared key.
    LegacyToken,
```

```rust
            ConfigError::LegacyToken => write!(
                f,
                "`token = \"...\"` is no longer read. Give each client its own entry under \
                 [tokens], for example `[tokens]` then `workshop-laptop = \"...\"`, so one can \
                 be revoked without locking out the rest"
            ),
```

And in `Config::load`, replace the token handling:

```rust
        if file.token.is_some() {
            return Err(ConfigError::LegacyToken);
        }
        let tokens: BTreeMap<String, String> = file
            .tokens
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, value)| !value.is_empty())
            .collect();
        if tokens.is_empty() {
            return Err(ConfigError::NoToken);
        }
```

Leave `ConfigError::NoToken`'s existing message as it is; it already says nothing would stop an unknown client starting a cut, which is still true.

- [ ] **Step 4: Match by name, and log which one**

In `crates/cut-host/src/serve.rs`, replace `token_matches` with:

```rust
/// The name of the token that matched, or `None`. Every candidate is compared in full, so the
/// time taken says nothing about how much of a token was right — and comparing all of them,
/// rather than stopping at the first match, keeps that true as clients are added.
pub fn match_token(presented: &str, tokens: &BTreeMap<String, String>) -> Option<String> {
    let mut matched: Option<String> = None;
    for (name, value) in tokens {
        if constant_time_eq(presented, value) {
            matched = Some(name.clone());
        }
    }
    matched
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}
```

Add `use std::collections::BTreeMap;` to the imports.

Then in `serve_client`, replace the token check so it names the client in the log:

```rust
    let presented: String = read_frame(&mut tls_stream, 1024, DEFAULT_BODY_TIMEOUT).map_err(io::Error::other)?;
    let Some(client) = match_token(&presented, tokens) else {
        eprintln!("cut host: {peer} presented a token matching no client");
        thread::sleep(std::time::Duration::from_secs(2));
        return Err(io::Error::other("bad token"));
    };
    eprintln!("cut host: {peer} authenticated as `{client}`");
```

`serve_client`'s `token: &str` parameter becomes `tokens: &BTreeMap<String, String>`, and `serve_on` passes `&config.tokens` instead of the `Arc<String>` it wraps today. Change the `Arc<String>` to `Arc<BTreeMap<String, String>>` and pass `&*tokens` at the call site.

Also change the dispatch log line so a revocation can be aimed correctly:

```rust
                if let Request::Dispatch { ref device, .. } = request {
                    eprintln!("cut host: `{client}` dispatched to {device}");
                }
```

`client` is bound before the request loop by the `let Some(client) = ...` above, so it is already in scope there — no extra binding is needed.

- [ ] **Step 5: Follow the change through the fixture and the daemon**

`crates/cut-host/tests/fixtures/mod.rs` builds a `Config` literal. Change its token field:

```rust
        tokens: [("test-client".to_string(), TOKEN.to_string())].into_iter().collect(),
```

`crates/cut-host/src/bin/cuthulhu-cutd.rs` prints what it loaded at startup. Add the client names, which is what an operator needs to revoke the right one:

```rust
    for name in config.tokens.keys() {
        eprintln!("cut host: client `{name}` may connect");
    }
```

Put it beside the existing device lines, after `Host::start`.

- [ ] **Step 6: Run the whole suite**

Run: `cargo test --workspace --locked`

Expected: PASS, including the end-to-end tests, which authenticate through the fixture's token.

- [ ] **Step 7: Update the install guide**

In `docs/cut-host.md`, replace the single-token config block with the table, keeping the surrounding prose consistent:

````markdown
```toml
# The address to listen on. A Cut Host refuses to start on a public address
# unless you pass --allow-public-bind, because a client can make a blade move.
bind = "192.168.1.50:7878"

# One token per client, named so you can tell them apart. Generate each with:
#   head -c 32 /dev/urandom | base64
# Revoking a client is deleting its line and restarting; the others keep working.
[tokens]
workshop-laptop = "REPLACE-ME"
```
````

And add a short section after the rotation one:

````markdown
## Revoking one client

Delete that client's line from `[tokens]` and restart:

```sh
sudo systemctl restart cuthulhu-cutd
```

Every other client keeps working. `journalctl -u cuthulhu-cutd` names which client authenticated
and which dispatched each Job, so you can tell which line to remove.
````

If the existing "Rotating the token" section says every paired desktop must be paired again, reword it: that is now only true of the client whose token changed.

- [ ] **Step 8: Verify the guide against the binary**

Run:

```sh
grep -n "token" docs/cut-host.md
cargo build -p cut-host --locked --bin cuthulhu-cutd
```

Read the config block you wrote and confirm it would load: a `[tokens]` table with at least one non-empty value, and no bare `token =` line left anywhere in the document.

- [ ] **Step 9: Commit**

```bash
git add crates/cut-host/src/config.rs crates/cut-host/src/serve.rs \
        crates/cut-host/src/bin/cuthulhu-cutd.rs crates/cut-host/tests/fixtures/mod.rs \
        docs/cut-host.md
git commit -m "Give each client its own token, so revoking one does not lock out the rest"
```

---

## Done when

- `cargo test --workspace --locked` passes, and `cargo test -p cut-host --locked --test end_to_end` passes three times running.
- `cargo build -p cut-host --locked --bin cuthulhu-cutd` succeeds and a clean rebuild is warning-free.
- `git diff main -- apps/ crates/cli/` is empty: this plan changes neither binary.
- `grep -rn "read_frame(" crates/cut-host/src/` shows every call passing a body timeout.
- `grep -n "^token" docs/cut-host.md` returns nothing — the old form is gone from the guide as well as the code.

## What phase 2's desktop half inherits

A `HostClient` that can be held open and polled without a hung Pi freezing the caller, and a
`cutd.toml` where each desktop has its own named token. The desktop plan can then pair a host,
store that token in `hosts.json`, and poll `Snapshot` on a timer — which is what the spec's
*Connections are held open* decision requires and what today's frame layer would not survive.
