# Security policy

## Reporting a vulnerability

While this repository is private, open an issue: only collaborators can read it,
so that is already a private channel. If the repository is ever made public,
turn on GitHub's private vulnerability reporting first and use that instead —
issues become world-readable at that moment, including any that are open.

Expect a best-effort response from a single maintainer, not a staffed queue.
Fixes land on `main`; there is no backport branch to wait for.

## What this software assumes

OA-Gateway is a prototype, and its security posture is the honest consequence of
that: **there is no authentication, no authorization, and no transport
security.** Any peer that can open a connection to an adapter port can publish
under any service identity, subscribe to any topic, and be believed. Nothing is
encrypted in transit, and nothing verifies that the broker on the other end is
the broker you meant.

So run it on loopback, or on a segment you control end to end, and put
authentication and TLS in front of it if you need them. `config/` binds to
loopback for this reason.

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
`owp.max_frame_size` are 16 MiB by default, and `owp.max_connections` and
`owp.max_subscriptions` bound how much state one caller can create. What remains
unbounded is the work done *after* a frame is accepted:

- **UCI XML and JSON conversion recurses without a depth limit**, so deep
  nesting is bounded only by the stack.
- **`maxOccurs` and `choice` are not enforced during conversion.** `maxOccurs`
  decides only whether a field becomes an array, never how many members it may
  hold, and an alternation converts as a set of optional siblings. A document can
  convert cleanly and still not be a valid instance of the standard: conversion
  is not validation.
- **Some conversion failures degrade silently rather than erroring**: an
  unparseable XML payload falls back to using the topic as its type hint, and a
  failed XML-to-JSON conversion forwards the raw XML to the subscriber.

## Supported versions

None, in the release sense. Every crate is `0.1.0`, nothing is published, and
there are no tags. `main` is the only supported thing, and its API may change
without notice.

## Dependencies

Roughly 110 packages come in transitively. `cargo-deny` enforces the policy in
`deny.toml` — RustSec advisories, yanked releases, a minimal license allow list,
and crates.io as the only source — and it runs weekly rather than only on push,
because advisories are published against code that has not changed. Dependabot
proposes the updates.

That covers what is known to the advisory database. A vulnerability you have
found in a dependency, and which is not published yet, still belongs in the
channel above.
