// Raw-body commands carry file bytes, which tauri-specta cannot describe. Their names and the
// header are shared here; `ImportOutcome` itself is generated in `bindings.ts`.

/** Takes the file bytes as the request body; returns `ImportOutcome`. */
export const IMPORT_COMMAND = "import_document";
/** Takes `{ id }`; returns the original file's bytes. */
export const FILE_COMMAND = "document_file";
/** URI-encoded original file name for `import_document`. */
export const FILE_NAME_HEADER = "x-file-name";
