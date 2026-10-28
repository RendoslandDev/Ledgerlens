# ADR 0001: Local-first SQLite storage

- Status: Accepted
- Date: 2026-09-30

## Context

Paperwork contains sensitive financial and personal information. The core workflow must function
without sending documents or extracted values to a hosted service. Durable ingestion and review
state also need transactional updates in one local database.

## Decision

Use SQLite for documents, records, and durable jobs. Keep source/model paths local and make network
integrations opt-in. Store monetary values as integer minor units with a currency code. Add vector
and full-text retrieval to the same database boundary when those milestones are implemented.

## Consequences

Backups and offline use are straightforward, and the job queue can share the local transaction
boundary. SQLite extensions and migrations need explicit versioning; concurrent multi-device writes
are not provided by the local database alone. Current vector persistence is a portable SQLite table,
not `sqlite-vec` yet.
