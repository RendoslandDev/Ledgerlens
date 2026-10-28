# Model artifacts

`manifest.json` is the tracked inventory. Add a model only after verifying its current version,
ONNX export, checksum, byte size, and license. Weight files are intentionally excluded from git;
downloaded artifacts are checked against the manifest by `ll-infer`.
