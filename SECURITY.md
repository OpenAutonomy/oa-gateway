# Security policy

## Reporting a vulnerability

Use GitHub's private vulnerability reporting: Security → Report a vulnerability.
**Do not open a public issue** for a vulnerability.

Expect a best-effort response from a single maintainer, not a staffed queue.
Fixes land on `main`; there is no backport branch to wait for.

## What this software assumes

OA-Gateway is a prototype, and its security posture is the honest consequence of
that: **there is no authentication, no authorization, and no in-process TLS.**
Any peer that can open a connection to an adapter port can publish under any
service identity, subscribe to any topic, and be believed. Nothing is encrypted
in transit, and nothing verifies that the broker on the other end is the broker
you meant.

Bind loopback, or put a reverse proxy in front that terminates TLS and
authentication. The host-oriented configs in `config/` bind to loopback for this
reason. A compose launch mounts whatever path you pass in `OAG_CONFIG`;
`config/compose.toml` listens on all interfaces inside the container so Docker
can publish the port, and the compose file still maps that port to `127.0.0.1`
on the host.

That assumption is what makes the rest of this document coherent. A finding is
interesting here if it lets a peer do something the posture above does *not*
already permit.

## In scope

- Memory unsafety. There is no `unsafe` in the workspace; anything that
  introduces it by way of a dependency or a new crate is worth reporting.
- A panic, hang, or unbounded loop reachable from a peer's frames or payloads.
  The STOMP decoder had exactly this bug — a hostile `content-length` panicked
  the process — and that class is treated as a defect, not a limitation.
- Mis-routing: a payload delivered to a subscriber whose route should not have
  matched, or a wrapper unwrapped into the wrong inner type. A gateway that
  quietly hands a message to the wrong peer is worse than one that refuses it.
- A codec turning a malformed frame into a plausible-but-wrong message instead
  of an error, particularly the A-GRA wrapper fields, which are located by
  substring search rather than by a parser.
- `scripts/fetch-uci-schema.sh` accepting a schema document that does not match
  the digests pinned in `scripts/uci-schema.sha256`.

## Known limitations, not vulnerabilities

These are real, they are understood, and under the posture above they tell you
only that an already-trusted peer can waste resources. They are hardening work
rather than security reports. Listing them here so nobody spends time proving
what is already written down.

Both edges now cap what a peer can hand them: `stomp.max_frame_size` and
`owp.max_frame_size` are 16 MiB by default, `owp.max_connections` and
`owp.max_subscriptions` bound how much state one caller can create, hex payloads
are capped before they are decoded, and conversion refuses nesting deeper than
96 elements. What remains is about correctness rather than resources:

- **Conversion still accepts what the standard does not.** `maxOccurs` decides
  only whether a field becomes an array, an alternation converts as a set of
  optional siblings, and an undeclared element is carried as a string. Those are
  now *reported* — `uci.validate` checks each payload against the compiled schema
  and defaults to `warn` where a schema is loaded — but warning is not refusing:
  a non-conforming payload still crosses unless the mode is `reject`.
- **A valid instance is not a fully checked instance.** Validation reads
  declarations, occurrence ranges, alternations, abstract types, the facets a type
  declares — enumerations, lengths, numeric bounds, patterns — and the lexical
  space of the primitive underneath, including the calendar, so `2026-02-30` is
  not a date and 99999999999999 is not an `xs:int`. Two things are still outside
  it. Identity constraints (`xs:unique`, `xs:key`) are not read, and neither is
  the co-occurrence logic a standard states in prose rather than in schema.
  Patterns from the corners of XSD's regex language that this build cannot
  translate are named in a startup warning rather than enforced, as are
  primitives it does not recognize; against the published catalog both lists are
  empty.
- **Conversion is best-effort in one direction that still matters.** A payload
  the engine carries in XML is converted for a JSON subscriber or the delivery is
  dropped and the client told, so nothing arrives in an unannounced format. What
  is not covered is the reverse question of whether a conversion that *succeeded*
  produced a valid instance of the standard — see the `maxOccurs` and `choice`
  note above.

## Supported versions

`0.1.0` on `main` is the supported line. The API may still change. There is no
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
