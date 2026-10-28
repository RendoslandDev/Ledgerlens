#![forbid(unsafe_code)]

use ll_core::{Error, Result};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::Read,
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestCandidate {
    pub path: PathBuf,
    pub sha256: String,
    pub size_bytes: u64,
}

pub fn scan_paths(paths: &[PathBuf]) -> Result<Vec<IngestCandidate>> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            collect_supported_files(path, &mut files)?;
        } else if path.is_file() {
            if !is_supported(path) {
                return Err(Error::Invalid(format!(
                    "unsupported document type: {}",
                    path.display()
                )));
            }
            files.push(path.clone());
        } else {
            return Err(Error::NotFound(path.display().to_string()));
        }
    }
    files.sort();
    files.dedup();
    files.into_iter().map(fingerprint).collect()
}

pub fn deduplicate(candidates: impl IntoIterator<Item = IngestCandidate>) -> Vec<IngestCandidate> {
    let mut candidates: Vec<_> = candidates.into_iter().collect();
    candidates.sort_by(|left, right| left.path.cmp(&right.path));
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| seen.insert(candidate.sha256.clone()))
        .collect()
}

fn collect_supported_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_supported_files(&path, files)?;
        } else if file_type.is_file() && is_supported(&path) {
            files.push(path);
        }
    }
    Ok(())
}

// ---- watched folders --------------------------------------------------------------------------

/// What a folder scan may touch. Everything outside these bounds is skipped and counted, never
/// read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FolderPolicy {
    /// Subfolder levels below the root that are searched (0 = the root only).
    pub max_depth: usize,
    /// Supported files listed per scan; the rest wait for a later scan.
    pub max_files: usize,
    /// Larger files are skipped without being opened.
    pub max_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundFile {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub modified_unix: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Skipped {
    /// Symbolic links (files or folders) are never followed.
    pub symlinks: u32,
    pub too_large: u32,
    /// Folders deeper than `max_depth`.
    pub too_deep: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FolderScan {
    pub files: Vec<FoundFile>,
    pub skipped: Skipped,
    /// More supported files exist than `max_files`.
    pub truncated: bool,
}

/// Validates a folder chosen for watching and returns its canonical path. The folder must be an
/// absolute path to a real directory; a symbolic link as the root is refused rather than followed.
pub fn check_folder(root: &Path) -> Result<PathBuf> {
    if !root.is_absolute() || root.components().any(|part| part == Component::ParentDir) {
        return Err(Error::Invalid("choose a folder by its full path".into()));
    }
    let metadata = fs::symlink_metadata(root)
        .map_err(|_| Error::NotFound("that folder no longer exists".into()))?;
    if metadata.file_type().is_symlink() {
        return Err(Error::Invalid("that folder is a link to another location; choose the folder itself".into()));
    }
    if !metadata.is_dir() {
        return Err(Error::Invalid("choose a folder, not a file".into()));
    }
    Ok(fs::canonicalize(root)?)
}

/// Lists supported documents under `root` within `policy`. Hidden files and folders are ignored,
/// symbolic links are not followed, and nothing is opened: only metadata is read.
pub fn scan_folder(root: &Path, policy: &FolderPolicy) -> Result<FolderScan> {
    let mut scan = FolderScan::default();
    walk(root, 0, policy, &mut scan)?;
    scan.files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(scan)
}

fn walk(directory: &Path, depth: usize, policy: &FolderPolicy, scan: &mut FolderScan) -> Result<()> {
    let mut entries: Vec<_> = fs::read_dir(directory)?.collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        // `DirEntry::file_type` does not follow links, so a link is seen as a link.
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            scan.skipped.symlinks += 1;
        } else if file_type.is_dir() {
            if depth >= policy.max_depth {
                scan.skipped.too_deep += 1;
            } else if let Err(error) = walk(&path, depth + 1, policy, scan) {
                // An unreadable subfolder does not stop the rest of the scan.
                if !matches!(error, Error::Io(_)) {
                    return Err(error);
                }
            }
        } else if file_type.is_file() && is_supported(&path) {
            let metadata = entry.metadata()?;
            if metadata.len() > policy.max_bytes {
                scan.skipped.too_large += 1;
            } else if scan.files.len() >= policy.max_files {
                scan.truncated = true;
            } else {
                let modified_unix = metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |elapsed| elapsed.as_secs() as i64);
                scan.files.push(FoundFile { path, size_bytes: metadata.len(), modified_unix });
            }
        }
    }
    Ok(())
}

/// Reads a regular file of at most `max_bytes`. The final path component is opened without
/// following links (so a file swapped for a link after the scan is refused), and the size is
/// enforced while reading, not just from metadata.
pub fn read_regular_file(path: &Path, max_bytes: u64) -> Result<Vec<u8>> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_FLAG_OPEN_REPARSE_POINT: a symlink or junction opens as itself, so the regular-file
        // check below refuses it instead of reading its target.
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(Error::Invalid("not a regular file".into()));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max_bytes {
        return Err(Error::Invalid("the file is larger than the size limit".into()));
    }
    Ok(bytes)
}

fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "pdf" | "png" | "jpg" | "jpeg" | "tif" | "tiff"
            )
        })
}

fn fingerprint(path: PathBuf) -> Result<IngestCandidate> {
    let mut file = File::open(&path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut size_bytes = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size_bytes = size_bytes
            .checked_add(read as u64)
            .ok_or_else(|| Error::Invalid(format!("file is too large: {}", path.display())))?;
        hasher.update(&buffer[..read]);
    }
    Ok(IngestCandidate {
        path,
        sha256: hex::encode(hasher.finalize()),
        size_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("ll-ingest-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn scans_supported_files_and_hashes_contents() {
        let root = temp_dir("scan");
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested/invoice.pdf"), b"invoice bytes").unwrap();
        fs::write(root.join("ignore.txt"), b"not a document").unwrap();

        let candidates = scan_paths(std::slice::from_ref(&root)).unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(
            candidates[0].sha256,
            "f3cbbf0fb1cedd39097a0159ee1b5e1a85fec1efcf8150399a6d25c3cf9739f3"
        );
        assert_eq!(candidates[0].size_bytes, 13);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn deduplication_is_content_based_and_path_deterministic() {
        let candidate = |path: &str| IngestCandidate {
            path: path.into(),
            sha256: "same".into(),
            size_bytes: 1,
        };
        let result = deduplicate([candidate("z.pdf"), candidate("a.pdf")]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, PathBuf::from("a.pdf"));
    }

    const POLICY: FolderPolicy = FolderPolicy { max_depth: 1, max_files: 10, max_bytes: 100 };

    #[test]
    fn folder_scans_stay_inside_their_policy() {
        let root = temp_dir("watch");
        fs::create_dir_all(root.join("2026/october/deeper")).unwrap();
        fs::create_dir_all(root.join(".hidden")).unwrap();
        fs::write(root.join("a.pdf"), b"%PDF-1.7").unwrap();
        fs::write(root.join("notes.txt"), b"ignored").unwrap();
        fs::write(root.join(".secret.pdf"), b"%PDF-1.7").unwrap();
        fs::write(root.join(".hidden/b.pdf"), b"%PDF-1.7").unwrap();
        fs::write(root.join("big.pdf"), vec![b'x'; 101]).unwrap();
        fs::write(root.join("2026/c.png"), b"png").unwrap();
        fs::write(root.join("2026/october/d.pdf"), b"%PDF-1.7").unwrap();

        let outside = temp_dir("watch-outside");
        fs::write(outside.join("private.pdf"), b"%PDF-1.7").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(outside.join("private.pdf"), root.join("link.pdf")).unwrap();
            std::os::unix::fs::symlink(&outside, root.join("linked-folder")).unwrap();
        }
        // Creating symlinks on Windows needs administrator rights or Developer Mode (CI has both).
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(outside.join("private.pdf"), root.join("link.pdf")).unwrap();
            std::os::windows::fs::symlink_dir(&outside, root.join("linked-folder")).unwrap();
        }

        let scan = scan_folder(&root, &POLICY).unwrap();
        let names: Vec<_> = scan.files.iter().map(|file| file.path.strip_prefix(&root).unwrap().to_path_buf()).collect();
        assert_eq!(names, [PathBuf::from("2026/c.png"), PathBuf::from("a.pdf")]);
        assert_eq!(scan.skipped.too_large, 1);
        assert_eq!(scan.skipped.too_deep, 1, "2026/october is below max_depth");
        #[cfg(any(unix, windows))]
        assert_eq!(scan.skipped.symlinks, 2);
        assert!(!scan.truncated);

        let limited = scan_folder(&root, &FolderPolicy { max_files: 1, ..POLICY }).unwrap();
        assert_eq!(limited.files.len(), 1);
        assert!(limited.truncated);

        #[cfg(any(unix, windows))]
        {
            assert!(check_folder(&root.join("linked-folder")).is_err(), "a linked root is refused");
            assert!(read_regular_file(&root.join("link.pdf"), 100).is_err(), "links are not opened");
        }
        assert!(check_folder(&root.join("a.pdf")).is_err());
        assert!(check_folder(Path::new("relative/folder")).is_err());
        assert!(check_folder(&root.join("2026/../2026")).is_err());
        assert_eq!(check_folder(&root).unwrap(), fs::canonicalize(&root).unwrap());

        assert_eq!(read_regular_file(&root.join("a.pdf"), 100).unwrap(), b"%PDF-1.7");
        assert!(read_regular_file(&root.join("big.pdf"), 100).is_err());
        assert!(read_regular_file(&root.join("2026"), 100).is_err());
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn explicit_unsupported_and_missing_paths_are_errors() {
        let root = temp_dir("errors");
        let text = root.join("notes.txt");
        fs::write(&text, b"notes").unwrap();
        assert!(scan_paths(std::slice::from_ref(&text)).is_err());
        assert!(scan_paths(&[root.join("missing.pdf")]).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
