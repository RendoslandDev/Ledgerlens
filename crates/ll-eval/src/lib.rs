pub mod cord;
pub mod score;

use ll_core::{Error, Result};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetKind {
    Sroie,
    Cord,
}

impl DatasetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sroie => "sroie",
            Self::Cord => "cord",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatasetSample {
    pub id: String,
    pub image_path: PathBuf,
    pub annotation_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DatasetInventory {
    pub schema_version: u32,
    pub dataset: DatasetKind,
    pub sample_count: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FieldCounts {
    pub true_positive: u64,
    pub false_positive: u64,
    pub false_negative: u64,
}

impl FieldCounts {
    pub fn precision(self) -> Option<f64> {
        let true_positive = self.true_positive as f64;
        let total = true_positive + self.false_positive as f64;
        (total > 0.0).then_some(true_positive / total)
    }

    pub fn recall(self) -> Option<f64> {
        let true_positive = self.true_positive as f64;
        let total = true_positive + self.false_negative as f64;
        (total > 0.0).then_some(true_positive / total)
    }

    pub fn f1(self) -> Option<f64> {
        let true_positive = self.true_positive as f64;
        let denominator =
            2.0 * true_positive + self.false_positive as f64 + self.false_negative as f64;
        (denominator > 0.0).then_some(2.0 * true_positive / denominator)
    }
}

pub fn load_sroie(root: impl AsRef<Path>) -> Result<Vec<DatasetSample>> {
    load_pairs(root.as_ref(), DatasetKind::Sroie)
}

pub fn load_cord(root: impl AsRef<Path>) -> Result<Vec<DatasetSample>> {
    load_pairs(root.as_ref(), DatasetKind::Cord)
}

fn load_pairs(root: &Path, kind: DatasetKind) -> Result<Vec<DatasetSample>> {
    if !root.is_dir() {
        return Err(Error::Invalid(format!(
            "dataset directory does not exist: {}",
            root.display()
        )));
    }

    let mut files = Vec::new();
    collect_files(root, &mut files)?;
    files.sort();

    let images: Vec<PathBuf> = files
        .iter()
        .filter(|path| is_image(path))
        .cloned()
        .collect();
    let annotations: Vec<PathBuf> = files
        .into_iter()
        .filter(|path| is_annotation(path, kind))
        .collect();
    let mut by_stem = BTreeMap::new();
    for annotation in annotations {
        let key = normalized_stem(&annotation)?;
        let priority = annotation_priority(&annotation, kind);
        match by_stem.get(&key) {
            Some((current_priority, _)) if *current_priority >= priority => {}
            _ => {
                by_stem.insert(key, (priority, annotation));
            }
        }
    }

    let mut samples = Vec::with_capacity(images.len());
    for image in images {
        let key = normalized_stem(&image)?;
        let (_, annotation_path) = by_stem
            .get(&key)
            .ok_or_else(|| Error::Invalid(format!("missing annotation for {}", image.display())))?;
        let relative = image.strip_prefix(root).unwrap_or(&image);
        let mut id = relative.to_string_lossy().replace('\\', "/");
        if let Some(extension) = Path::new(&id).extension().and_then(|value| value.to_str()) {
            id.truncate(id.len() - extension.len() - 1);
        }
        samples.push(DatasetSample {
            id,
            image_path: image,
            annotation_path: annotation_path.clone(),
        });
    }

    if samples.is_empty() {
        return Err(Error::Invalid(format!(
            "no labeled {} samples found under {}",
            kind.as_str(),
            root.display()
        )));
    }
    Ok(samples)
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_files(&path, files)?;
        } else if entry.file_type()?.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn is_image(path: &Path) -> bool {
    matches!(
        extension(path).as_deref(),
        Some("jpg" | "jpeg" | "png" | "tif" | "tiff")
    )
}

fn is_annotation(path: &Path, kind: DatasetKind) -> bool {
    if has_component(path, &["box", "boxes"]) {
        return false;
    }
    match kind {
        DatasetKind::Cord => extension(path).as_deref() == Some("json"),
        DatasetKind::Sroie => {
            let in_label_dir = has_component(path, &["entities", "annotations", "labels"]);
            matches!(extension(path).as_deref(), Some("json"))
                || (extension(path).as_deref() == Some("txt")
                    && (in_label_dir || !has_component(path, &["img", "image", "images"])))
        }
    }
}

fn annotation_priority(path: &Path, kind: DatasetKind) -> u8 {
    if has_component(path, &["entities", "annotations", "labels"]) {
        return 3;
    }
    match (kind, extension(path).as_deref()) {
        (DatasetKind::Cord, Some("json")) | (DatasetKind::Sroie, Some("json")) => 2,
        (DatasetKind::Sroie, Some("txt")) => 1,
        _ => 0,
    }
}

fn extension(path: &Path) -> Option<String> {
    path.extension()?.to_str().map(str::to_ascii_lowercase)
}

fn normalized_stem(path: &Path) -> Result<String> {
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| Error::Invalid(format!("invalid dataset filename: {}", path.display())))
}

fn has_component(path: &Path, names: &[&str]) -> bool {
    path.ancestors()
        .filter_map(Path::file_name)
        .filter_map(|part| part.to_str())
        .any(|part| names.iter().any(|name| part.eq_ignore_ascii_case(name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("ll-eval-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn sroie_loader_uses_entities_not_box_annotations() {
        let root = temp_dir("sroie");
        fs::create_dir_all(root.join("train/img")).unwrap();
        fs::create_dir_all(root.join("train/entities")).unwrap();
        fs::create_dir_all(root.join("train/box")).unwrap();
        fs::write(root.join("train/img/receipt.jpg"), b"image").unwrap();
        fs::write(root.join("train/entities/receipt.txt"), b"ground truth").unwrap();
        fs::write(root.join("train/box/receipt.txt"), b"word boxes").unwrap();

        let samples = load_sroie(&root).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].id, "train/img/receipt");
        assert!(samples[0].annotation_path.ends_with("entities/receipt.txt"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cord_loader_pairs_images_with_json_annotations() {
        let root = temp_dir("cord");
        fs::create_dir_all(root.join("train")).unwrap();
        fs::write(root.join("train/receipt_01.png"), b"image").unwrap();
        fs::write(root.join("train/receipt_01.json"), b"{}").unwrap();

        let samples = load_cord(&root).unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].id, "train/receipt_01");
        assert!(samples[0].annotation_path.ends_with("receipt_01.json"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn field_f1_uses_true_positives_and_both_error_counts() {
        let counts = FieldCounts {
            true_positive: 8,
            false_positive: 2,
            false_negative: 2,
        };
        assert_eq!(counts.precision(), Some(0.8));
        assert_eq!(counts.recall(), Some(0.8));
        assert_eq!(counts.f1(), Some(0.8));
        assert_eq!(FieldCounts::default().f1(), None);
    }
}
