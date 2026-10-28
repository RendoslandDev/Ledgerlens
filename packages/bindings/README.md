# Bindings

TypeScript types and command wrappers for the desktop app, imported as `@ledgerlens/bindings`.

- `src/bindings.ts` is **generated** by `tauri-specta` from `apps/desktop/src-tauri` and the
  boundary types in `crates/ll-app` (ADR 0003). Do not edit it by hand.
- `src/raw.ts` names the two raw-body commands (`import_document`, `document_file`). They carry
  file bytes, which `specta` cannot describe; their result type (`ImportOutcome`) is still
  generated.

After changing a command or a boundary type:

```bash
npm --prefix apps/desktop run bindings
```

`npm --prefix apps/desktop run check:bindings` (also part of `cargo test` in `src-tauri`) fails
when the committed file no longer matches the Rust code.

64-bit integers are never exported implicitly. Each such field in `ll-app` opts in to a
TypeScript `number` with `specta_typescript::Number`, and amounts entered by a person are capped
at `ll_app::MAX_SAFE_MINOR` (2^53 - 1) so they stay exact in JavaScript.
