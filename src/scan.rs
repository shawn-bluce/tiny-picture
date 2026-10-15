//! Discovers image files from dropped paths.
//!
//! Images are detected by file extension only; file contents are never read.

use std::fs;
use std::path::{Path, PathBuf};

/// File extensions (lowercase) treated as images.
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif"];

/// Returns true if the path has a supported image extension (case-insensitive).
pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| IMAGE_EXTENSIONS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

/// Collects images from a dropped path.
///
/// A file is returned as-is if it is an image. A directory is scanned
/// recursively; hidden entries are skipped and symlinked directories are not
/// followed. Results are sorted by path.
pub fn collect_images(path: &Path) -> Vec<PathBuf> {
    let mut images = Vec::new();
    if path.is_dir() {
        walk_dir(path, &mut images);
        images.sort();
    } else if path.is_file() && is_image(path) {
        images.push(path.to_path_buf());
    }
    images
}

fn walk_dir(dir: &Path, images: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        // `DirEntry::file_type` does not follow symlinks, which prevents cycles.
        let Ok(file_type) = entry.file_type() else { continue };
        let path = entry.path();
        if file_type.is_dir() {
            walk_dir(&path, images);
        } else if is_image(&path) && path.is_file() {
            images.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tiny-picture-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn detects_images_by_extension() {
        assert!(is_image(Path::new("a.jpg")));
        assert!(is_image(Path::new("a.JPEG")));
        assert!(is_image(Path::new("dir/a.Png")));
        assert!(!is_image(Path::new("a.txt")));
        assert!(!is_image(Path::new("jpg")));
        assert!(!is_image(Path::new("a")));
    }

    #[test]
    fn scans_directories_recursively() {
        let dir = temp_dir("scan");
        touch(&dir.join("b.png"));
        touch(&dir.join("notes.txt"));
        touch(&dir.join("nested/deeper/a.JPG"));
        touch(&dir.join(".hidden/c.png"));
        touch(&dir.join(".d.png"));

        let found = collect_images(&dir);
        assert_eq!(found, vec![dir.join("b.png"), dir.join("nested/deeper/a.JPG")]);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn accepts_single_image_file_only() {
        let dir = temp_dir("single");
        touch(&dir.join("a.webp"));
        touch(&dir.join("a.txt"));

        assert_eq!(collect_images(&dir.join("a.webp")), vec![dir.join("a.webp")]);
        assert!(collect_images(&dir.join("a.txt")).is_empty());
        assert!(collect_images(&dir.join("missing.png")).is_empty());

        fs::remove_dir_all(&dir).unwrap();
    }
}
