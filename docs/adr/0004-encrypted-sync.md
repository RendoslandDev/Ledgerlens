# ADR 0004: Authenticated optional sync

- Status: Accepted
- Date: 2026-09-30

## Context

Some users may want multiple devices, but a relay must not learn document contents or operation
patches. Sync is optional and must not weaken the local-only default.

## Decision

Represent sync changes as an operation log and encrypt each serialized operation with
XChaCha20-Poly1305 before transport. Bind the envelope version as associated data and reject tables
outside an explicit allow-list. Keep relay transport and key management out of the local core.

## Consequences

Authenticated encryption provides confidentiality and tamper detection for an envelope, not a
complete sync protocol. Key provisioning, replay protection, device lifecycle, conflict resolution,
and relay authentication must be designed and reviewed before any network transport is enabled.
