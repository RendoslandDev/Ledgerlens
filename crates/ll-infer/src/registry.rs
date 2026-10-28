use crate::ep::ExecutionProvider;
use ll_core::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};

const MAX_MODEL_MANIFEST_BYTES: u64 = 1_048_576;
const MAX_MODEL_ARTIFACT_BYTES: u64 = 16 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Task { OcrDetect, OcrRecognize, Embed, Classify, Extract }

/// Describes one model artifact. Manifests are `<name>.model.json` files next to the weights.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSpec {
    pub name: String,
    pub version: String,
    pub task: Task,
    /// Weights file, relative to the manifest's directory.
    pub file: String,
    #[serde(default)]
    pub sha256: Option<String>,
    #[serde(default)]
    #[serde(rename = "size")]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub license: Option<String>,
    /// Per-item input shape (without batch dimension); 0 marks a dynamic axis.
    #[serde(default)]
    pub input_shape: Vec<usize>,
    /// Empty = any execution provider.
    #[serde(default)]
    pub supported_eps: Vec<ExecutionProvider>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelManifest {
    pub schema_version: u32,
    pub models: Vec<ModelSpec>,
}

#[derive(Debug, Default)]
pub struct ModelRegistry {
    models: BTreeMap<String, RegisteredModel>,
}

#[derive(Debug)]
struct RegisteredModel {
    spec: ModelSpec,
    base: PathBuf,
    path: PathBuf,
}

impl ModelRegistry {
    pub fn new() -> Self { Self::default() }

    /// Registers a spec; `base` is the directory used to resolve `spec.file`.
    pub fn register(&mut self, spec: ModelSpec, base: &Path) -> Result<()> {
        if spec.name.is_empty() { return Err(Error::Invalid("model name is empty".into())); }
        if spec.version.is_empty() || spec.file.is_empty() {
            return Err(Error::Invalid(format!("model {} needs a version and file", spec.name)));
        }
        let relative_path = Path::new(&spec.file);
        if spec.file.contains('\\')
            || relative_path.is_absolute()
            || !relative_path.components().all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(Error::Invalid(format!("model {} file must be a safe relative path", spec.name)));
        }
        let key = format!("{}@{}", spec.name, spec.version);
        if self.models.contains_key(&key) { return Err(Error::Invalid(format!("duplicate model {key}"))); }
        let path = base.join(relative_path);
        self.models.insert(key, RegisteredModel { spec, base: base.to_path_buf(), path });
        Ok(())
    }

    /// Loads every `*.model.json` in `dir`. Returns how many were registered.
    pub fn discover(&mut self, dir: &Path) -> Result<usize> {
        if !dir.exists() { return Ok(0); }
        let path = dir.join("manifest.json");
        if !path.exists() { return Ok(0); }
        let metadata = fs::metadata(&path)?;
        if metadata.len() > MAX_MODEL_MANIFEST_BYTES {
            return Err(Error::Invalid("model manifest exceeds the size limit".into()));
        }
        let mut manifest_bytes = Vec::new();
        File::open(&path)?.take(MAX_MODEL_MANIFEST_BYTES + 1).read_to_end(&mut manifest_bytes)?;
        if manifest_bytes.len() as u64 > MAX_MODEL_MANIFEST_BYTES {
            return Err(Error::Invalid("model manifest exceeds the size limit".into()));
        }
        let manifest: ModelManifest = serde_json::from_slice(&manifest_bytes)
            .map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))?;
        if manifest.schema_version != 1 {
            return Err(Error::Invalid(format!("unsupported model manifest schema {}", manifest.schema_version)));
        }
        for spec in &manifest.models {
            if spec.sha256.as_deref().is_none_or(|sha| sha.len() != 64 || !sha.bytes().all(|byte| byte.is_ascii_hexdigit()))
                || spec.size_bytes.is_none_or(|size| size == 0 || size > MAX_MODEL_ARTIFACT_BYTES)
                || spec.license.as_deref().is_none_or(str::is_empty)
            {
                return Err(Error::Invalid(format!("model {} is missing checksum, size, or license", spec.name)));
            }
        }
        let count = manifest.models.len();
        for spec in manifest.models { self.register(spec, dir)?; }
        Ok(count)
    }

    fn latest(&self, name: &str) -> Option<&RegisteredModel> {
        self.models
            .iter()
            .filter(|(_, model)| model.spec.name == name)
            .max_by(|a, b| a.1.spec.version.cmp(&b.1.spec.version))
            .map(|(_, model)| model)
    }

    /// Latest registered version (lexicographic on the version string) of `name`.
    pub fn get(&self, name: &str) -> Option<&ModelSpec> {
        self.latest(name).map(|model| &model.spec)
    }

    pub fn by_task(&self, task: Task) -> Vec<&ModelSpec> {
        self.models.values().filter(|model| model.spec.task == task).map(|model| &model.spec).collect()
    }

    pub fn len(&self) -> usize { self.models.len() }
    pub fn is_empty(&self) -> bool { self.models.is_empty() }

    /// Checks the artifact's declared size and checksum.
    pub fn verify(&self, name: &str) -> Result<()> {
        self.verified_model(name).map(|_| ())
    }

    /// Returns a canonical artifact path only after all trust checks have passed.
    pub fn verified_model(&self, name: &str) -> Result<(&ModelSpec, PathBuf)> {
        let model = self.latest(name).ok_or_else(|| Error::NotFound(format!("model {name}")))?;
        let expected_size = model.spec.size_bytes
            .filter(|size| *size > 0 && *size <= MAX_MODEL_ARTIFACT_BYTES)
            .ok_or_else(|| Error::Invalid(format!("model {name} has no valid declared size")))?;
        let expected_hash = model.spec.sha256.as_deref()
            .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .ok_or_else(|| Error::Invalid(format!("model {name} has no valid SHA-256")))?;

        let base = fs::canonicalize(&model.base).map_err(Error::storage)?;
        let path = fs::canonicalize(&model.path).map_err(Error::storage)?;
        if !path.starts_with(&base) {
            return Err(Error::Invalid(format!("model {name} resolves outside its model directory")));
        }
        let metadata = fs::metadata(&path).map_err(Error::storage)?;
        if !metadata.is_file() {
            return Err(Error::Invalid(format!("model {name} artifact is not a regular file")));
        }
        if metadata.len() != expected_size {
            return Err(Error::Inference(format!("size mismatch for {name}: expected {expected_size}, got {}", metadata.len())));
        }

        let mut file = File::open(&path).map_err(Error::storage)?;
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 64 * 1024];
        let mut total = 0u64;
        loop {
            let read = file.read(&mut buffer).map_err(Error::storage)?;
            if read == 0 { break; }
            total = total.checked_add(read as u64)
                .ok_or_else(|| Error::Inference(format!("model {name} size overflows")))?;
            if total > expected_size {
                return Err(Error::Inference(format!("model {name} changed size while being verified")));
            }
            hasher.update(&buffer[..read]);
        }
        if total != expected_size {
            return Err(Error::Inference(format!("model {name} changed size while being verified")));
        }
        let actual_hash = hex::encode(hasher.finalize());
        if !actual_hash.eq_ignore_ascii_case(expected_hash) {
            return Err(Error::Inference(format!("checksum mismatch for {name}: expected {expected_hash}, got {actual_hash}")));
        }
        Ok((&model.spec, path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(name: &str, version: &str, file: &str, sha: Option<String>) -> ModelSpec {
        ModelSpec {
            name: name.into(), version: version.into(), task: Task::Embed, file: file.into(),
            sha256: sha, size_bytes: None, license: None, input_shape: vec![], supported_eps: vec![],
        }
    }
    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("ll-infer-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn latest_version_wins() {
        let mut r = ModelRegistry::new();
        r.register(spec("m", "1.0", "a", None), Path::new("/x")).unwrap();
        r.register(spec("m", "1.2", "b", None), Path::new("/x")).unwrap();
        assert_eq!(r.get("m").unwrap().version, "1.2");
        assert!(r.register(spec("m", "1.2", "b", None), Path::new("/x")).is_err());
    }

    #[test]
    fn registration_rejects_absolute_and_traversal_paths() {
        let mut registry = ModelRegistry::new();
        for file in ["../outside.onnx", "/tmp/outside.onnx", "nested\\..\\outside.onnx"] {
            assert!(registry.register(spec("unsafe", file, file, None), Path::new("/models")).is_err());
        }
    }

    #[test]
    fn discover_and_verify() {
        let d = tmp("disc");
        std::fs::write(d.join("w.onnx"), b"weights").unwrap();
        let sha = hex::encode(Sha256::digest(b"weights"));
        let model = ModelSpec {
            size_bytes: Some(7),
            license: Some("Apache-2.0".into()),
            ..spec("emb", "1", "w.onnx", Some(sha))
        };
        let manifest = ModelManifest { schema_version: 1, models: vec![model] };
        std::fs::write(d.join("manifest.json"), serde_json::to_string(&manifest).unwrap()).unwrap();
        let mut r = ModelRegistry::new();
        assert_eq!(r.discover(&d).unwrap(), 1);
        let (spec, path) = r.verified_model("emb").unwrap();
        assert_eq!(spec.name, "emb");
        assert!(path.starts_with(std::fs::canonicalize(&d).unwrap()));
        assert_eq!(r.by_task(Task::Embed).len(), 1);
        std::fs::write(d.join("w.onnx"), b"too large").unwrap();
        assert!(r.verify("emb").is_err());
        std::fs::write(d.join("w.onnx"), b"changed").unwrap();
        assert!(r.verify("emb").is_err());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn verification_rejects_symlinks_escaping_the_model_directory() {
        use std::os::unix::fs::symlink;

        let directory = tmp("symlink-root");
        let outside = tmp("symlink-outside").join("weights.onnx");
        std::fs::write(&outside, b"weights").unwrap();
        symlink(&outside, directory.join("linked.onnx")).unwrap();
        let sha = hex::encode(Sha256::digest(b"weights"));
        let model = ModelSpec {
            size_bytes: Some(7),
            license: Some("Apache-2.0".into()),
            ..spec("linked", "1", "linked.onnx", Some(sha))
        };
        let mut registry = ModelRegistry::new();
        registry.register(model, &directory).unwrap();
        assert!(registry.verify("linked").is_err());
        std::fs::remove_dir_all(directory).unwrap();
        std::fs::remove_dir_all(outside.parent().unwrap()).unwrap();
    }

    #[test]
    fn missing_dir_is_empty() {
        assert_eq!(ModelRegistry::new().discover(Path::new("/definitely/not/here")).unwrap(), 0);
    }

    #[test]
    fn discovers_repository_model_manifest_without_fake_entries() {
        let models = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models");
        assert_eq!(ModelRegistry::new().discover(&models).unwrap(), 0);
    }
}
