//! User-facing compression options and output path rules.

use std::path::{Path, PathBuf};

/// Prefix added to output file names when `replace` is disabled.
pub const OUTPUT_PREFIX: &str = "tiny_";

/// Compression level, ordered from best quality to smallest size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompressionLevel {
    Lossless,
    Ultra,
    High,
    Medium,
    Tiny,
}

impl CompressionLevel {
    pub const ALL: [Self; 5] = [Self::Lossless, Self::Ultra, Self::High, Self::Medium, Self::Tiny];

    /// Maps a slider position to a level, clamping out-of-range values.
    pub fn from_index(index: i32) -> Self {
        Self::ALL[index.clamp(0, Self::ALL.len() as i32 - 1) as usize]
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Lossless => "Lossless",
            Self::Ultra => "Ultra",
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Tiny => "Tiny",
        }
    }

    /// Whether pixels must stay unchanged.
    pub fn is_lossless(self) -> bool {
        self == Self::Lossless
    }

    /// Quality (0-100) for lossy encoders (JPEG, WebP). `Lossless` maps to the
    /// highest quality, used only when the target format has no lossless mode.
    pub fn quality(self) -> f32 {
        match self {
            Self::Lossless => 95.0,
            Self::Ultra => 90.0,
            Self::High => 80.0,
            Self::Medium => 70.0,
            Self::Tiny => 55.0,
        }
    }

    /// Palette size for lossy PNG (quantization), or `None` to keep full color.
    pub fn png_colors(self) -> Option<usize> {
        match self {
            Self::Lossless | Self::Ultra => None,
            Self::High => Some(256),
            Self::Medium => Some(128),
            Self::Tiny => Some(64),
        }
    }
}

/// Target format for the "Convert to" option.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputFormat {
    Original,
    Jpg,
    Png,
}

impl OutputFormat {
    pub const ALL: [Self; 3] = [Self::Original, Self::Jpg, Self::Png];

    /// Maps a combo box index to a format, falling back to `Original`.
    pub fn from_index(index: i32) -> Self {
        usize::try_from(index).ok().and_then(|i| Self::ALL.get(i).copied()).unwrap_or(Self::Original)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Original => "Original",
            Self::Jpg => "JPG",
            Self::Png => "PNG",
        }
    }

    /// File extensions belonging to this format; the first one is preferred.
    fn extensions(self) -> &'static [&'static str] {
        match self {
            Self::Original => &[],
            Self::Jpg => &["jpg", "jpeg"],
            Self::Png => &["png"],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CompressOptions {
    pub level: CompressionLevel,
    pub format: OutputFormat,
    /// Use as many system resources as possible to finish quickly.
    pub powerful: bool,
    /// Overwrite the source file instead of writing a prefixed copy next to it.
    pub replace: bool,
}

/// Computes where the compressed version of `source` is written.
///
/// - The extension changes only when converting to a different format; a
///   source that already matches the target keeps its own extension
///   (e.g. `.jpeg` stays `.jpeg` when converting to JPG).
/// - With `replace` disabled, the file name gets [`OUTPUT_PREFIX`].
pub fn output_path(source: &Path, options: &CompressOptions) -> PathBuf {
    let source_ext = source.extension().and_then(|e| e.to_str()).unwrap_or_default();
    let target_exts = options.format.extensions();
    let ext = if target_exts.is_empty() || target_exts.iter().any(|e| source_ext.eq_ignore_ascii_case(e)) {
        source_ext
    } else {
        target_exts[0]
    };

    let stem = source.file_stem().map(|s| s.to_string_lossy()).unwrap_or_default();
    let prefix = if options.replace { "" } else { OUTPUT_PREFIX };
    let file_name = if ext.is_empty() { format!("{prefix}{stem}") } else { format!("{prefix}{stem}.{ext}") };
    source.with_file_name(file_name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(format: OutputFormat, replace: bool) -> CompressOptions {
        CompressOptions { level: CompressionLevel::High, format, powerful: false, replace }
    }

    #[test]
    fn level_from_index_clamps() {
        assert_eq!(CompressionLevel::from_index(-1), CompressionLevel::Lossless);
        assert_eq!(CompressionLevel::from_index(2), CompressionLevel::High);
        assert_eq!(CompressionLevel::from_index(99), CompressionLevel::Tiny);
    }

    #[test]
    fn format_from_index_falls_back_to_original() {
        assert_eq!(OutputFormat::from_index(1), OutputFormat::Jpg);
        assert_eq!(OutputFormat::from_index(-1), OutputFormat::Original);
        assert_eq!(OutputFormat::from_index(7), OutputFormat::Original);
    }

    #[test]
    fn replace_keeps_source_path() {
        let src = Path::new("/a/b/photo.PNG");
        assert_eq!(output_path(src, &options(OutputFormat::Original, true)), src);
        assert_eq!(output_path(src, &options(OutputFormat::Png, true)), src);
    }

    #[test]
    fn no_replace_adds_prefix_in_same_folder() {
        let src = Path::new("/a/b/photo.jpg");
        assert_eq!(output_path(src, &options(OutputFormat::Original, false)), Path::new("/a/b/tiny_photo.jpg"));
    }

    #[test]
    fn conversion_changes_extension() {
        let src = Path::new("/a/photo.webp");
        assert_eq!(output_path(src, &options(OutputFormat::Jpg, true)), Path::new("/a/photo.jpg"));
        assert_eq!(output_path(src, &options(OutputFormat::Png, false)), Path::new("/a/tiny_photo.png"));
        // Already a JPEG: keep the original `.jpeg` spelling.
        let jpeg = Path::new("/a/photo.jpeg");
        assert_eq!(output_path(jpeg, &options(OutputFormat::Jpg, true)), jpeg);
    }
}
