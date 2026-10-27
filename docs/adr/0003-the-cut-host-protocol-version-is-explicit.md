<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# The Cut Host protocol version is explicit

A desktop and a Cut Host agree one protocol version before the token, and before any `Request`,
`Response`, or `Event`. The version is `cut_host::protocol::PROTOCOL_VERSION`. It is not the
crate version and not the desktop's version. Those move when the program does. This number moves
when a frame changes meaning.

This build speaks exactly that one version. It does not speak a range, and it does not keep the
unversioned session working beside it. A peer that offers anything else is refused by name and
the connection closes. No Job is admitted, no dispatch id is recorded, and no cutter is claimed.

## What is agreed

After TLS, and before the token:

1. The client writes one frame: a JSON integer, its `PROTOCOL_VERSION`. The frame is capped at
   `VERSION_FRAME_MAX` (128 bytes), far under the cap a dispatch may use.
2. The server reads that frame as a `u32` and nothing else. The read shares the unauthenticated
   budget (`PRE_AUTH_TIMEOUT`) with the token read that follows a success, so the extra frame
   does not extend how long an unauthenticated peer can hold a slot.
3. If the integer is `PROTOCOL_VERSION`, the server writes `VersionReply::Agreed` and only then
   reads the token. Ordinary requests follow the greeting, as before.
4. If the integer is some other version, the server writes `VersionReply::Unsupported` carrying
   the offered number and the number it speaks, and closes. It does not read the token.
5. If the frame is not a version — the wrong shape, or a length past the version cap — the
   server writes `VersionReply::Unreadable` and closes. The body of an oversized frame is not
   read. A `Request` is never decoded from this frame.

`VersionReply` carries no job, no device, and no token. `deny_unknown_fields` keeps a payload
from riding along in it. The agreement frames are not themselves versioned: a peer has to decode
them in order to learn that it must refuse, so changing their shape is a new protocol, not a
bump of this number.

## Compatibility

- Equal versions proceed. Anything else is a refusal. There is no "best we both speak".
- A change to `Request`, `Response`, `Event`, or a type they carry — a field added, removed,
  renamed, or reinterpreted, where a peer must understand it to cut safely — increments
  `PROTOCOL_VERSION`. The previous number is not accepted. `DeviceSnapshot.claimed` is why: a
  desktop that required it and a host that omitted it failed as a deserialization error, with
  nothing naming which side was old.
- A field that already has a documented default inside the current version (the omitted
  `CutStatus` job id reads as no job) stays part of that version's shape. A new incompatibility
  is a new number, not another silent default.
- The client sends no token until `Agreed` comes back. A mismatch therefore does not present a
  credential.
- When the host names a version, the desktop's sentence follows the two numbers. A higher host
  version means this Cuthulhu is older. A lower one means the Cut Host is older.
- A host that completes TLS and then closes without a version reply is the unversioned protocol
  this replaced. The desktop reports it as an older Cut Host. A timeout, with no reply and no
  close, stays a reachability failure: the peer has not shown what it speaks.
- An operator updates the older side. On a Pi that is replacing `/usr/local/bin/cuthulhu-cutd`
  and restarting the unit, which waits for a Job already in progress. A Job the host already
  owns keeps cutting; a new one is not accepted until the versions match.

## Considered options

**A version byte on every frame** was rejected. Agreement is once per connection. A header on
every frame still has to be understood by a peer that does not know the version yet, which is
the problem, moved.

**Accept the old session when the first frame is a string** was rejected. That is the ambiguous
fallback. The failure this exists for is a peer that looks almost compatible and then breaks
inside a snapshot. A token-shaped first frame is `Unreadable`, and the dispatch behind it is
not read.

**A range, or a list of features** was rejected. Out of scope, and it is how a missing field
gets through as a default. One number, equal or refuse.

## Consequences

Client, server, the scripted host fixture, and the tests speak the versioned session only. A
replay of the unversioned session (token frame, then a dispatch) is refused and leaves every
cutter idle.

The desktop shows the mismatch where a host's connection error already appears: the sentence
from `ClientError`, unchanged, on the host's row. The code is `host_protocol`.
