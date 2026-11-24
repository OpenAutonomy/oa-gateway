# Security policy

## Reporting a vulnerability

Use GitHub's private vulnerability reporting: Security → Report a vulnerability.
**Do not open a public issue** for a vulnerability.

Expect a best-effort response from a single maintainer, not a staffed queue.
Fixes land on `main`; there is no backport branch to wait for.

## What this software assumes

OA-Gateway is a prototype, and its security posture is the honest consequence of
that: **there is no authorization, and authentication exists only as an
opt-in exception on one adapter.** By default, any peer that can open a
connection to an adapter port can publish under any service identity,
subscribe to any topic, and be believed.

TLS is available on OWP and STOMP, and off by default on both. `owp.tls_cert`
and `owp.tls_key` make the OWP listener terminate TLS, so a client connects
with `wss://` instead of plaintext `ws://`. `stomp.tls` makes the STOMP client
dial the broker over TLS instead of plaintext, verifying the broker's
certificate against `stomp.tls_ca` or the operating system trust store.
Unset (the default for both), each is exactly what it always was. Encryption
is not authentication by itself: a peer or broker that completes a TLS
handshake with nothing further configured is still believed about who it is
and what it may do, the same as a plaintext one — and `login`/`passcode`
remain what authenticates the gateway to the broker, not the TLS handshake
itself. DDS has no TLS — its RTPS transport is UDP, not a TCP stream, so the
TLS available to OWP and STOMP could never apply to it as-is; DDS Security is
the separate standard for that, and this build does not configure it.

OWP's TLS listener has one further, opt-in step past that: setting
`owp.tls_client_ca` requires and verifies a client certificate from that CA
on every connection, refusing anything else at the handshake. This is the
one real exception to "no authentication" for a peer connecting *to* this
gateway — off by default, and when off the posture above is unchanged.
Turning it on still does not turn on authorization: a client that presents a
valid certificate is not treated any differently from one that connected
before `owp.tls_client_ca` was set. It may publish and subscribe exactly as
any peer could before, and the gateway does not record or act on which
certificate it was — the verification happens once, at the handshake, and
nothing about the identity survives past it.

STOMP's `tls_client_cert`/`tls_client_key` run the other direction: the
gateway presenting its own certificate to the broker, so a broker whose SSL
transport connector requires one (ActiveMQ's `needClientAuth`) accepts the
connection. This authenticates the gateway *to* the broker — the same role
`login`/`passcode` already play, just via TLS instead of a STOMP header — and
has no bearing on whether a peer can authenticate to this gateway. Off by
default, and requires `tls = true` to matter at all.

Bind loopback, or put a reverse proxy in front that authenticates callers and
decides what they may do. In-process TLS covers encryption and, for OWP with
`tls_client_ca` set, proof that a connecting client holds a certificate this
gateway was told to trust — it does not decide what that client may do once
connected, which is what "no authorization" means here. The host-oriented
configs in `config/` bind to loopback for this reason. A compose launch mounts
whatever path you pass in `OAG_CONFIG`; `config/compose.toml` listens on all
interfaces inside the container so Docker can publish the port, and the
compose file still maps that port to `127.0.0.1` on the host.

The OWP listener does not check the `Origin` header by default, so a browser
on any page can open a WebSocket to it wherever it is reachable. `owp.allowed_origins`
is an opt-in allowlist that refuses a handshake from an unlisted origin; it is
not a substitute for the proxy, since a non-browser client sets whatever
`Origin` it likes, but it closes the cross-site-WebSocket path for browser
clients. Empty by default, so the posture above is unchanged until it is set.

That assumption is what makes the rest of this document coherent. A finding is
interesting here if it lets a peer do something the posture above does *not*
already permit.

## In scope

- Memory unsafety. There is no `unsafe` in this workspace's own code — every
  crate sets `unsafe_code = "forbid"`. The one deliberate, reviewed exception
  is the FFI boundary a dependency crosses: `oa-gateway-xsd-libxml` links
  libxml2 (via the `libxml` crate) to validate UCI/A-GRA payloads against
  their real XSD, and that `unsafe` lives inside libxml2 and its Rust
  bindings, not in any crate here. Anything that introduces `unsafe` by way
  of a *different* dependency or a new crate, beyond that one accepted
  boundary, is worth reporting.
- A panic, hang, or unbounded loop reachable from a peer's frames or payloads.
  The STOMP decoder had exactly this bug — a hostile `content-length` panicked
  the process — and that class is treated as a defect, not a limitation.
- Mis-routing: a payload delivered to a subscriber whose route should not have
  matched, or a wrapper unwrapped into the wrong inner type. A gateway that
  quietly hands a message to the wrong peer is worse than one that refuses it.
- A codec turning a malformed frame into a plausible-but-wrong message instead
  of an error. The A-GRA wrapper fields had exactly this bug — they were
  located by substring search, so a sibling field's free text containing the
  literal text of a tag name could anchor extraction to the wrong place — and
  that class is treated as a defect, not a limitation. They are parsed with
  `roxmltree` now, a real parser rather than substring search.
- `scripts/fetch-uci-schema.sh` or `scripts/fetch-agra-schema.sh` accepting a
  schema document that does not match the digests pinned in
  `scripts/uci-schema.sha256` or `scripts/agra-schema.sha256`, respectively.

## Known limitations, not vulnerabilities

These are real, they are understood, and under the posture above they tell you
only that an already-trusted peer can waste resources. They are hardening work
rather than security reports. Listing them here so nobody spends time proving
what is already written down.

Every edge now caps what a peer can hand it: `stomp.max_frame_size`,
`owp.max_frame_size`, and `dds.max_sample_size` are 16 MiB by default,
`owp.max_connections` and `owp.max_subscriptions` bound how much state one
caller can create, and hex payloads are capped before they are decoded.
Nothing separately caps how deeply nested a payload's own XML elements are —
only its overall byte size is — so parsing or validating a pathologically
deep but small document relies on `roxmltree`'s and libxml2's own limits,
not anything this gateway enforces itself. What remains is about
correctness rather than resources:

- **`uci.validate` defaults to `warn`, not `reject`.** Warning is not
  refusing: a non-conforming payload still crosses unless the mode is
  `reject`.
- **A valid instance is not a fully checked instance.** Validation is backed by
  libxml2 (`oa-gateway-xsd-libxml`), a general-purpose XSD 1.0 engine, not a
  hand-rolled subset — so it is not approximating the facet and pattern
  language, it implements it, and it also checks identity constraints
  (`xs:unique`, `xs:key`), which an earlier, hand-rolled validator in this
  codebase did not. One thing is still outside it, and always will be for any
  schema-only check: the co-occurrence logic a standard states in prose
  ("field B is required when field A is X") rather than in the schema itself
  has nothing in the XSD for a validator to read, libxml2 included.

## Supported versions

`0.3.0` on `main` is the supported line. The API may still change. There is no
backport branch.

## Dependencies

Roughly 110 packages come in transitively. `cargo-deny` enforces the policy in
`deny.toml` — RustSec advisories, yanked releases, a minimal license allow list,
and crates.io as the only source — and it runs weekly rather than only on push,
because advisories are published against code that has not changed. Dependabot
proposes the updates.

That covers what is known to the advisory database. A vulnerability you have
found in a dependency, and which is not published yet, still belongs in the
channel above.
