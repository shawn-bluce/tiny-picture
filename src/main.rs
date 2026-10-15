mod compress;
mod jpegtran;
mod options;
mod scan;

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::thread;

use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{Model, ModelRc, SharedString, VecModel};

use crate::compress::{Event, Outcome};
use crate::options::{CompressOptions, CompressionLevel, OUTPUT_PREFIX, OutputFormat};

slint::include_modules!();

/// Shown in size columns that have no value yet.
const NO_VALUE: &str = "—";

fn main() -> Result<(), slint::PlatformError> {
    // Drag and drop is handled through winit window events, so require the winit backend.
    slint::BackendSelector::new().backend_name("winit".into()).select()?;

    let ui = AppWindow::new()?;
    ui.set_files(ModelRc::new(VecModel::<FileItem>::default()));
    ui.set_output_prefix(OUTPUT_PREFIX.into());
    update_idle_status(&ui);

    install_drop_handler(&ui);

    ui.on_clear_files({
        let ui = ui.as_weak();
        move || {
            let Some(ui) = ui.upgrade() else { return };
            with_files(&ui, |files| files.set_vec(Vec::new()));
            ui.set_progress(0.0);
            update_idle_status(&ui);
        }
    });

    ui.on_start_compress({
        let ui = ui.as_weak();
        move || {
            if let Some(app) = ui.upgrade() {
                start_compress(&app);
            }
        }
    });

    ui.run()
}

fn with_files<R>(ui: &AppWindow, f: impl FnOnce(&VecModel<FileItem>) -> R) -> R {
    let model = ui.get_files();
    let files = model.as_any().downcast_ref::<VecModel<FileItem>>().expect("files must be a VecModel");
    f(files)
}

/// Files that Compress will process: new ones and ones that failed before.
fn needs_compression(item: &FileItem) -> bool {
    matches!(item.status, FileStatus::Pending | FileStatus::Failed)
}

fn image_count(n: usize) -> String {
    if n == 1 { "1 image".into() } else { format!("{n} images") }
}

/// Formats a byte count in kilobytes (1 KB = 1000 bytes, like Finder).
fn format_kb(bytes: i64) -> String {
    format!("{:.1} KB", bytes as f64 / 1000.0)
}

/// Formats `saved / original` as a percentage with one decimal.
fn format_percent(saved: i64, original: u64) -> String {
    let percent = if original == 0 { 0.0 } else { saved as f64 * 100.0 / original as f64 };
    format!("{percent:.1}%")
}

/// Updates the pending count and, unless a job is running, the status text.
fn update_idle_status(ui: &AppWindow) {
    let (total, pending) = with_files(ui, |files| (files.row_count(), files.iter().filter(needs_compression).count()));
    ui.set_pending_count(pending as i32);
    let status = match (total, pending) {
        (0, _) => "No images".to_string(),
        (_, 0) => format!("{} · Drop more to add them", image_count(total)),
        _ => format!("{}, {pending} to compress · Drop more to add them", image_count(total)),
    };
    ui.set_status(status.into());
}

/// Forwards Finder drag and drop events to the UI.
fn install_drop_handler(ui: &AppWindow) {
    let weak = ui.as_weak();
    ui.window().on_winit_window_event(move |_, event| {
        let Some(ui) = weak.upgrade() else { return EventResult::Propagate };
        match event {
            WindowEvent::HoveredFile(_) => ui.set_drag_over(!ui.get_busy()),
            WindowEvent::HoveredFileCancelled => ui.set_drag_over(false),
            WindowEvent::DroppedFile(path) => {
                ui.set_drag_over(false);
                if !ui.get_busy() {
                    scan_in_background(&ui, path.clone());
                }
            }
            _ => return EventResult::Propagate,
        }
        EventResult::PreventDefault
    });
}

/// Scans a dropped path and reads file sizes off the UI thread, then appends
/// the found images to the list.
fn scan_in_background(ui: &AppWindow, path: PathBuf) {
    ui.set_scanning(ui.get_scanning() + 1);
    let weak = ui.as_weak();
    thread::spawn(move || {
        let images: Vec<(PathBuf, u64)> = scan::collect_images(&path)
            .into_iter()
            .map(|image| {
                let size = fs::metadata(&image).map_or(0, |m| m.len());
                (image, size)
            })
            .collect();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.set_scanning(ui.get_scanning() - 1);
            add_files(&ui, images);
        });
    });
}

/// Appends images that are not in the list yet.
fn add_files(ui: &AppWindow, images: Vec<(PathBuf, u64)>) {
    with_files(ui, |files| {
        let mut known: HashSet<String> = files.iter().map(|f| f.path.to_string()).collect();
        for (image, size) in images {
            let path = image.to_string_lossy().into_owned();
            if known.insert(path.clone()) {
                files.push(file_item(&image, path, size));
            }
        }
    });
    ui.set_progress(0.0);
    update_idle_status(ui);
}

fn file_item(image: &Path, path: String, size: u64) -> FileItem {
    let name = image.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let folder = image.parent().map(display_folder).unwrap_or_default();
    let mut item = FileItem {
        name: name.into(),
        folder: folder.into(),
        path: path.into(),
        original_size: format_kb(size as i64).into(),
        ..Default::default()
    };
    reset_result(&mut item);
    item
}

/// Marks a file as waiting and clears the result of a previous run.
fn reset_result(item: &mut FileItem) {
    let empty = SharedString::from(NO_VALUE);
    item.status = FileStatus::Pending;
    item.message = SharedString::default();
    item.compressed_size = empty.clone();
    item.saved_size = empty.clone();
    item.saved_percent = empty;
}

/// Shortens the home directory to `~` for display.
fn display_folder(folder: &Path) -> String {
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from)
        && let Ok(rest) = folder.strip_prefix(&home)
    {
        return Path::new("~").join(rest).display().to_string();
    }
    folder.display().to_string()
}

fn read_options(ui: &AppWindow) -> CompressOptions {
    CompressOptions {
        level: CompressionLevel::from_index(ui.get_level().round() as i32),
        format: OutputFormat::from_index(ui.get_convert_index()),
        powerful: ui.get_powerful(),
        replace: ui.get_replace(),
    }
}

/// Updates one list row with a compression event.
fn apply_event(item: &mut FileItem, event: Event) {
    match event {
        Event::Started(_) => item.status = FileStatus::Compressing,
        Event::Finished(_, Ok(Outcome::Compressed { original_size, compressed_size })) => {
            let saved = original_size as i64 - compressed_size as i64;
            item.status = FileStatus::Done;
            item.original_size = format_kb(original_size as i64).into();
            item.compressed_size = format_kb(compressed_size as i64).into();
            item.saved_size = format_kb(saved).into();
            item.saved_percent = format_percent(saved, original_size).into();
        }
        Event::Finished(_, Ok(Outcome::Skipped { reason })) => {
            item.status = FileStatus::Skipped;
            item.message = reason.into();
        }
        Event::Finished(_, Err(error)) => {
            item.status = FileStatus::Failed;
            item.message = error.into();
        }
    }
}

/// Compresses the files that are waiting or failed before. Files that are
/// already compressed or skipped are left alone, so appending new files and
/// pressing Compress again never re-compresses earlier results.
fn start_compress(ui: &AppWindow) {
    let options = read_options(ui);
    let (rows, paths): (Vec<usize>, Vec<PathBuf>) = with_files(ui, |files| {
        let mut rows = Vec::new();
        let mut paths = Vec::new();
        for (row, mut item) in files.iter().enumerate() {
            if needs_compression(&item) {
                paths.push(PathBuf::from(item.path.as_str()));
                reset_result(&mut item);
                files.set_row_data(row, item);
                rows.push(row);
            }
        }
        (rows, paths)
    });
    let total = paths.len();
    if total == 0 {
        return;
    }

    let mut details = options.level.label().to_string();
    if options.format != OutputFormat::Original {
        details += &format!(", converting to {}", options.format.label());
    }
    ui.set_busy(true);
    ui.set_progress(0.0);
    ui.set_status(format!("Compressing 0 of {total} ({details})…").into());

    let weak = ui.as_weak();
    thread::spawn(move || {
        let finished = AtomicUsize::new(0);
        let skipped = AtomicUsize::new(0);
        let failed = AtomicUsize::new(0);
        let original_bytes = AtomicU64::new(0);
        let compressed_bytes = AtomicU64::new(0);

        compress::run(&paths, options, |event| {
            let (index, done) = match &event {
                Event::Started(index) => (*index, finished.load(Ordering::Relaxed)),
                Event::Finished(index, result) => {
                    match result {
                        Ok(Outcome::Compressed { original_size, compressed_size }) => {
                            original_bytes.fetch_add(*original_size, Ordering::Relaxed);
                            compressed_bytes.fetch_add(*compressed_size, Ordering::Relaxed);
                        }
                        Ok(Outcome::Skipped { .. }) => _ = skipped.fetch_add(1, Ordering::Relaxed),
                        Err(_) => _ = failed.fetch_add(1, Ordering::Relaxed),
                    }
                    (*index, finished.fetch_add(1, Ordering::Relaxed) + 1)
                }
            };
            let row = rows[index];
            let details = details.clone();
            let _ = weak.upgrade_in_event_loop(move |ui| {
                with_files(&ui, |files| {
                    if let Some(mut item) = files.row_data(row) {
                        apply_event(&mut item, event);
                        files.set_row_data(row, item);
                    }
                });
                ui.set_progress(done as f32 / total as f32);
                ui.set_status(format!("Compressing {done} of {total} ({details})…").into());
            });
        });

        let (skipped, failed) = (skipped.into_inner(), failed.into_inner());
        let compressed = total - skipped - failed;
        let (before, after) = (original_bytes.into_inner(), compressed_bytes.into_inner());
        let saved = before as i64 - after as i64;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.set_busy(false);
            update_idle_status(&ui);
            let mut summary = format!("Done · {} compressed", image_count(compressed));
            if compressed > 0 {
                summary += &format!(", saved {} ({})", format_kb(saved), format_percent(saved, before));
            }
            if skipped > 0 {
                summary += &format!(" · {skipped} skipped");
            }
            if failed > 0 {
                summary += &format!(" · {failed} failed");
            }
            ui.set_status(summary.into());
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_kilobytes_with_one_decimal() {
        assert_eq!(format_kb(0), "0.0 KB");
        assert_eq!(format_kb(93_691), "93.7 KB");
        assert_eq!(format_kb(1_234_567), "1234.6 KB");
        assert_eq!(format_kb(-2_500), "-2.5 KB");
    }

    #[test]
    fn formats_percentages_with_one_decimal() {
        assert_eq!(format_percent(31_772, 93_691), "33.9%");
        assert_eq!(format_percent(0, 0), "0.0%");
        assert_eq!(format_percent(-500, 10_000), "-5.0%");
    }
}
