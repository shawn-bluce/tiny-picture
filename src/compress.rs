//! Compresses images and runs compression jobs on a pool of worker threads.
//!
//! Codecs: mozjpeg (JPEG, plus lossless optimization in [`crate::jpegtran`]),
//! oxipng + exoquant (PNG), libwebp (WebP).
//! Decoding is done by the `image` crate.

use std::fs;
use std::io::Cursor;
use std::num::NonZero;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageReader, Rgb, RgbImage};

use crate::options::{CompressOptions, CompressionLevel, OutputFormat, output_path};

/// Progress reported for the file at the given index.
#[derive(Debug)]
pub enum Event {
    Started(usize),
    /// Holds the outcome on success, or an error message.
    Finished(usize, Result<Outcome, String>),
}

/// Result of compressing a single file.
#[derive(Debug)]
pub enum Outcome {
    /// The compressed file was written.
    Compressed { original_size: u64, compressed_size: u64 },
    /// The file is already compressed well, so nothing was written.
    Skipped { reason: String },
}

/// Lossy re-encoding to the same format must save at least this much to be
/// worth the quality loss; otherwise the file counts as already compressed.
const MIN_LOSSY_SAVING_PERCENT: usize = 5;

/// Number of worker threads: all cores in powerful mode, half of them otherwise.
pub fn worker_count(powerful: bool) -> usize {
    let cores = thread::available_parallelism().map_or(1, NonZero::get);
    if powerful { cores } else { (cores / 2).max(1) }
}

/// Compresses `files` in parallel, blocking until all of them are processed.
///
/// `on_event` is called from worker threads.
pub fn run(files: &[PathBuf], options: CompressOptions, on_event: impl Fn(Event) + Sync) {
    let next = AtomicUsize::new(0);
    let workers = worker_count(options.powerful).min(files.len());
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(file) = files.get(index) else { break };
                    on_event(Event::Started(index));
                    on_event(Event::Finished(index, compress_file(file, &options)));
                }
            });
        }
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Format {
    Jpeg,
    Png,
    Webp,
    Gif,
}

impl Format {
    fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        match ext.as_str() {
            "jpg" | "jpeg" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "webp" => Some(Self::Webp),
            "gif" => Some(Self::Gif),
            _ => None,
        }
    }
}

/// Compresses a single file and writes the result to [`output_path`].
///
/// When the format stays the same, files that are already compressed well are
/// skipped and left untouched: lossless optimization must save something, and
/// lossy re-encoding must save at least [`MIN_LOSSY_SAVING_PERCENT`].
/// Format conversions are always written.
pub fn compress_file(source: &Path, options: &CompressOptions) -> Result<Outcome, String> {
    let input = fs::read(source).map_err(|e| format!("Couldn't read the file: {e}"))?;
    let source_format = Format::from_path(source).ok_or("Unsupported file type")?;
    let target_format = match options.format {
        OutputFormat::Original => source_format,
        OutputFormat::Jpg => Format::Jpeg,
        OutputFormat::Png => Format::Png,
    };
    let same_format = source_format == target_format;
    let level = options.level;
    let lossless = match target_format {
        Format::Jpeg => same_format && level.is_lossless(),
        Format::Png => level.png_colors().is_none(),
        Format::Webp | Format::Gif => level.is_lossless(),
    };

    let compressed = match target_format {
        // Lossless JPEG optimization re-encodes the existing coefficients.
        Format::Jpeg if same_format && level.is_lossless() => crate::jpegtran::optimize(&input)?,
        // Lossless PNG optimization works on the file directly and keeps 16-bit depth.
        Format::Png if same_format && level.png_colors().is_none() => {
            oxipng::optimize_from_memory(&input, &png_options()).map_err(|e| e.to_string())?
        }
        Format::Gif => return Err("GIF files can't be compressed yet. Convert them to PNG or JPG instead.".into()),
        _ => {
            let (image, icc) = decode(&input)?;
            match target_format {
                Format::Jpeg => encode_jpeg(&image, icc.as_deref(), level.quality())?,
                Format::Png => encode_png(&image, icc.as_deref(), level.png_colors())?,
                Format::Webp => encode_webp(&image, level),
                Format::Gif => unreachable!("handled above"),
            }
        }
    };

    if same_format {
        let saved = input.len().saturating_sub(compressed.len());
        let worth_it = if lossless { saved > 0 } else { saved * 100 >= input.len() * MIN_LOSSY_SAVING_PERCENT };
        if !worth_it {
            let reason = if saved == 0 {
                "Already compressed, it wouldn't get any smaller".to_string()
            } else {
                format!(
                    "Already compressed, it would only get {:.1}% smaller",
                    saved as f64 * 100.0 / input.len() as f64
                )
            };
            return Ok(Outcome::Skipped { reason });
        }
    }

    let output = output_path(source, options);
    write_atomically(&output, &compressed).map_err(|e| format!("Couldn't save the file: {e}"))?;
    Ok(Outcome::Compressed { original_size: input.len() as u64, compressed_size: compressed.len() as u64 })
}

/// Decodes an image, applying its EXIF orientation. Returns the ICC profile, if any.
fn decode(data: &[u8]) -> Result<(DynamicImage, Option<Vec<u8>>), String> {
    let error = |e: image::ImageError| format!("Couldn't decode the image: {e}");
    let reader = ImageReader::new(Cursor::new(data)).with_guessed_format().map_err(|e| e.to_string())?;
    let mut decoder = reader.into_decoder().map_err(error)?;
    let icc = decoder.icc_profile().ok().flatten();
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder).map_err(error)?;
    image.apply_orientation(orientation);
    Ok((image, icc))
}

fn encode_jpeg(image: &DynamicImage, icc: Option<&[u8]>, quality: f32) -> Result<Vec<u8>, String> {
    let rgb = flatten_alpha(image);
    // mozjpeg reports errors by unwinding.
    panic::catch_unwind(AssertUnwindSafe(|| -> std::io::Result<Vec<u8>> {
        let mut compress = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
        compress.set_size(rgb.width() as usize, rgb.height() as usize);
        compress.set_quality(quality);
        let mut started = compress.start_compress(Vec::new())?;
        if let Some(icc) = icc {
            started.write_icc_profile(icc);
        }
        started.write_scanlines(rgb.as_raw())?;
        started.finish()
    }))
    .map_err(|_| "JPEG encoding failed".to_string())?
    .map_err(|e| format!("JPEG encoding failed: {e}"))
}

/// JPEG has no alpha channel: composite transparent pixels onto white.
fn flatten_alpha(image: &DynamicImage) -> RgbImage {
    if !image.color().has_alpha() {
        return image.to_rgb8();
    }
    let rgba = image.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let over_white = |c: u8| ((c as u32 * a as u32 + 255 * (255 - a as u32)) / 255) as u8;
        Rgb([over_white(r), over_white(g), over_white(b)])
    })
}

/// Encodes a PNG, reduced to a dithered palette of `colors` when given.
fn encode_png(image: &DynamicImage, icc: Option<&[u8]>, colors: Option<usize>) -> Result<Vec<u8>, String> {
    use oxipng::{BitDepth, ColorType, RGBA8, RawImage};

    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let raw = match colors {
        None => RawImage::new(width, height, ColorType::RGBA, BitDepth::Eight, rgba.into_raw()),
        Some(colors) => {
            let pixels: Vec<_> = rgba.pixels().map(|p| exoquant::Color::new(p[0], p[1], p[2], p[3])).collect();
            let (palette, indices) = exoquant::convert_to_indexed(
                &pixels,
                width as usize,
                colors,
                &exoquant::optimizer::KMeans,
                &exoquant::ditherer::FloydSteinberg::new(),
            );
            let palette = palette.iter().map(|c| RGBA8::new(c.r, c.g, c.b, c.a)).collect();
            RawImage::new(width, height, ColorType::Indexed { palette }, BitDepth::Eight, indices)
        }
    };
    let mut raw = raw.map_err(|e| e.to_string())?;
    if let Some(icc) = icc {
        raw.add_icc_profile(icc);
    }
    raw.create_optimized_png(&png_options()).map_err(|e| e.to_string())
}

fn png_options() -> oxipng::Options {
    oxipng::Options {
        // Drop metadata that doesn't affect how the image looks (keeps color profiles).
        strip: oxipng::StripChunks::Safe,
        ..oxipng::Options::from_preset(2)
    }
}

fn encode_webp(image: &DynamicImage, level: CompressionLevel) -> Vec<u8> {
    let rgba = image.to_rgba8();
    let encoder = webp::Encoder::from_rgba(&rgba, rgba.width(), rgba.height());
    let memory = if level.is_lossless() { encoder.encode_lossless() } else { encoder.encode(level.quality()) };
    memory.to_vec()
}

/// Writes through a temporary file and a rename, so a failure never leaves a
/// half-written file behind (important when replacing originals).
fn write_atomically(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let temp = path.with_file_name(format!(".{file_name}.tiny-picture.tmp"));
    let result = fs::write(&temp, data)
        .and_then(|()| match fs::metadata(path) {
            Ok(existing) => fs::set_permissions(&temp, existing.permissions()),
            Err(_) => Ok(()),
        })
        .and_then(|()| fs::rename(&temp, path));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use image::{ImageFormat, RgbaImage};

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tiny-picture-compress-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A smooth gradient with a semi-transparent area, saved in the format of `path`.
    fn write_sample(path: &Path, alpha: bool) {
        let image = RgbaImage::from_fn(96, 64, |x, y| {
            let a = if alpha && x < 32 { 128 } else { 255 };
            image::Rgba([(x * 2) as u8, (y * 3) as u8, ((x + y) % 256) as u8, a])
        });
        let image = DynamicImage::ImageRgba8(image);
        let image = if alpha { image } else { DynamicImage::ImageRgb8(image.to_rgb8()) };
        image.save(path).unwrap();
    }

    fn options(level: CompressionLevel, format: OutputFormat, replace: bool) -> CompressOptions {
        CompressOptions { level, format, powerful: false, replace }
    }

    /// Returns `(original, compressed)` sizes of a file that must have been written.
    fn sizes(outcome: Outcome) -> (u64, u64) {
        match outcome {
            Outcome::Compressed { original_size, compressed_size } => (original_size, compressed_size),
            Outcome::Skipped { reason } => panic!("unexpectedly skipped: {reason}"),
        }
    }

    #[test]
    fn lossless_png_keeps_pixels() {
        let dir = temp_dir("lossless-png");
        let file = dir.join("a.png");
        write_sample(&file, true);
        let before = image::open(&file).unwrap().to_rgba8();

        let outcome = compress_file(&file, &options(CompressionLevel::Lossless, OutputFormat::Original, true)).unwrap();

        let (original, compressed) = sizes(outcome);
        assert!(compressed < original);
        assert_eq!(image::open(&file).unwrap().to_rgba8(), before);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lossy_png_writes_prefixed_copy() {
        let dir = temp_dir("lossy-png");
        let file = dir.join("a.png");
        write_sample(&file, true);
        let original = fs::read(&file).unwrap();

        let outcome = compress_file(&file, &options(CompressionLevel::Tiny, OutputFormat::Original, false)).unwrap();

        assert_eq!(fs::read(&file).unwrap(), original, "original must be untouched");
        let output = image::open(dir.join("tiny_a.png")).unwrap();
        assert_eq!((output.width(), output.height()), (96, 64));
        let (original, compressed) = sizes(outcome);
        assert!(compressed < original);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn converts_transparent_png_to_jpeg() {
        let dir = temp_dir("png-to-jpg");
        let file = dir.join("a.png");
        write_sample(&file, true);

        let outcome = compress_file(&file, &options(CompressionLevel::High, OutputFormat::Jpg, true)).unwrap();

        assert!(sizes(outcome).1 > 0);
        assert!(file.exists(), "conversion keeps the source file");
        let bytes = fs::read(dir.join("a.jpg")).unwrap();
        assert_eq!(image::guess_format(&bytes).unwrap(), ImageFormat::Jpeg);
        let output = image::load_from_memory(&bytes).unwrap().to_rgb8();
        // The half-transparent area is blended onto white, so it gets lighter.
        assert!(output.get_pixel(0, 0)[0] > 100);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn lossless_jpeg_keeps_pixels() {
        let dir = temp_dir("jpeg");
        let file = dir.join("a.jpg");
        write_sample(&file, false);
        let before = image::open(&file).unwrap().to_rgb8();

        let outcome = compress_file(&file, &options(CompressionLevel::Lossless, OutputFormat::Original, true)).unwrap();

        let (original, compressed) = sizes(outcome);
        assert!(compressed < original);
        assert_eq!(image::open(&file).unwrap().to_rgb8(), before);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn skips_already_compressed_files() {
        let dir = temp_dir("skip");
        for (name, level) in [("a.jpg", CompressionLevel::Medium), ("b.png", CompressionLevel::Lossless)] {
            let file = dir.join(name);
            write_sample(&file, false);
            let opts = options(level, OutputFormat::Original, true);

            sizes(compress_file(&file, &opts).unwrap());
            let once = fs::read(&file).unwrap();
            let again = compress_file(&file, &opts).unwrap();

            assert!(matches!(again, Outcome::Skipped { .. }), "{name}: {again:?}");
            assert_eq!(fs::read(&file).unwrap(), once, "{name}: a skipped file must be untouched");
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn conversion_is_never_skipped() {
        let dir = temp_dir("convert");
        let file = dir.join("a.jpg");
        write_sample(&file, false);

        // JPEG to PNG makes this file larger, but the user asked for PNG.
        let outcome = compress_file(&file, &options(CompressionLevel::Lossless, OutputFormat::Png, false)).unwrap();

        assert!(matches!(outcome, Outcome::Compressed { .. }));
        assert!(dir.join("tiny_a.png").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn recompresses_webp() {
        let dir = temp_dir("webp");
        let file = dir.join("a.webp");
        // Photo-like noise: lossy WebP beats the lossless original here.
        // (A smooth gradient would be skipped, since lossless WebP is already tiny.)
        let mut seed = 1u32;
        let noisy = image::RgbImage::from_fn(128, 128, |x, y| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            let noise = (seed >> 24) as u8 / 4;
            image::Rgb([(x * 2) as u8 / 2 + noise, (y * 2) as u8 / 2 + noise, noise * 2])
        });
        noisy.save(&file).unwrap();

        let outcome = compress_file(&file, &options(CompressionLevel::Medium, OutputFormat::Original, true)).unwrap();

        let (original, compressed) = sizes(outcome);
        assert!(compressed < original);
        let bytes = fs::read(&file).unwrap();
        assert_eq!(image::guess_format(&bytes).unwrap(), ImageFormat::WebP);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gif_reports_an_error() {
        let dir = temp_dir("gif");
        let file = dir.join("a.gif");
        write_sample(&file, false);

        assert!(compress_file(&file, &options(CompressionLevel::High, OutputFormat::Original, true)).is_err());
        assert!(compress_file(&file, &options(CompressionLevel::High, OutputFormat::Png, true)).is_ok());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn processes_every_file_once() {
        let dir = temp_dir("run");
        let files: Vec<PathBuf> = (0..6).map(|i| dir.join(format!("{i}.png"))).collect();
        files.iter().for_each(|f| write_sample(f, false));
        let missing = dir.join("missing.png");
        let mut all = files.clone();
        all.push(missing);

        let finished = Mutex::new(Vec::new());
        run(&all, options(CompressionLevel::High, OutputFormat::Original, true), |event| {
            if let Event::Finished(index, result) = event {
                finished.lock().unwrap().push((index, result.is_ok()));
            }
        });

        let mut finished = finished.into_inner().unwrap();
        finished.sort();
        let expected: Vec<_> = (0..7).map(|i| (i, i < 6)).collect();
        assert_eq!(finished, expected);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn powerful_mode_uses_at_least_as_many_workers() {
        assert!(worker_count(true) >= worker_count(false));
        assert!(worker_count(false) >= 1);
    }
}
