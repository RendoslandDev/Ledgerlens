# Threat model

## Scope and assets

Ledgerlens handles receipts, invoices, contracts, statements, extracted personal/financial data,
model artifacts, and optional synchronization operations. The intended default is one local user
and one local database. This document covers the Rust foundation, not the future Tauri UI or relay.

## Trust boundaries

- Imported paths and file contents are untrusted, even when selected by the user.
- OCR output, model output, and model metadata are untrusted until validated.
- SQLite is a local persistence boundary; another process running as the same OS user can still
  access the database unless operating-system protections are configured.
- Export files leave the application-controlled database and inherit the destination's access
  controls.
- Sync ciphertext may cross a network in a future relay, but key management and transport are not
  implemented here.

## Current controls

- File ingestion limits candidates to known document extensions, hashes bytes with SHA-256, and
  deduplicates identical content. Parsers are isolated in threads, not sandboxed in a separate
  process.
- Extraction validates totals, currencies, and dates; uncertain or inconsistent values are routed
  for review instead of silently treated as correct.
- Documents are admitted by magic bytes (PDF, PNG, JPEG, TIFF) and capped at 25 MB. PDF text
  parsing and OCR run on isolated worker threads with time limits, so a crash is contained and a
  hang is abandoned; the document is marked unreadable.
- Imports are read by a background worker through the durable `ll-jobs` queue. A crash or quit
  mid-read re-queues the document on the next start; storage errors retry with backoff (3
  attempts), and a stored file that no longer matches its SHA-256 fails at once. The jobs table
  records only error codes, never error messages, because messages can quote document content.
  Any import still waiting or being read can be cancelled; a cancelled read's result is discarded.
- Folder watching is off by default. It is enabled only by choosing a folder in the operating
  system's folder dialog; no command accepts a folder path from the web view. The whole home
  folder, `~/Library`, system locations, top-level folders such as `/Users`, and the app's own
  data folder are refused. Symbolic links are never followed (a linked root is refused, links
  inside the tree are skipped, and files are opened with `O_NOFOLLOW`). Scans list at most 2,000
  files, 3 subfolder levels deep, skip hidden files and files over 25 MB, and read at most 10
  files / 100 MB per 15-second scan. Files are copied in; nothing in the folder is modified. The
  settings page shows the folder, these limits, and what each scan skipped. Watching can be
  paused or stopped at any time.
- OCR (macOS) uses Apple's on-device Vision framework; pages are rasterised by Quartz with a
  16-megapixel cap and at most 10 pages per document. No document content is sent off the device.
  OCR'd records skip review only when subtotal + tax = total corroborates the amounts.
- Structured query values are bound parameters. Query shape is selected from a closed enum.
- Model loading checks the declared artifact size and checksum. A checksum detects mismatch but
  does not authenticate who published the manifest.
- Sync payloads use XChaCha20-Poly1305 with a fresh OS-random nonce, a versioned envelope, and
  associated data. Ciphertext tampering and wrong keys fail authentication.
- Sync operations are restricted to an explicit table allow-list before encryption and after
  decryption.

## Known gaps and required controls before release

- Database files, source documents, and exports are not encrypted at rest by this foundation.
  On iOS and Android the app's storage is sandboxed and encrypted by the operating system while
  the device is locked; on desktop it is not. Phones are lost more often than desktops, so
  Ledgerlens's own encryption at rest (roadmap Phase 6) matters more before the mobile release.
- On phones, documents arrive only through the system picker (camera, Photos, Files), which the
  person opens; there is no folder access and no folder watching. Exports go to the app's own
  Documents folder, which the Files app shows.
- No secure key provisioning, rotation, recovery, device revocation, replay protection, or
  conflict-resolution protocol exists for sync. Do not connect the envelope to a relay yet.
- A local SHA-256 manifest is not a signature. Model provenance and license review remain required;
  only checksums from a trusted source should be accepted.
- PDF parsing/rendering and native inference are not sandboxed. Apply resource limits and isolate
  parsers before processing arbitrary external documents at scale.
- Folder watching polls one person-chosen folder within fixed limits (see above). It assumes one
  app instance per data folder: on start, jobs left running are treated as interrupted. Running
  two instances on the same data folder is not supported or prevented yet.
- Avoid logging document text, extracted fields, SQL parameter values, keys, or plaintext sync
  operations. Add redaction tests when application logging is introduced.
- Exports can expose financial data. The desktop app now shows the target path and row count and
  writes only after an explicit confirm; keep that step for any future export or share target.

## Security review triggers

Revisit this model before enabling network sync, adding a model downloader, adding unattended file
watching, handling multiple OS users, or shipping installers. Each trigger changes a trust boundary
or introduces a new credential/data lifecycle.
