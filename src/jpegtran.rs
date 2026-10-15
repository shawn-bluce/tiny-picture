//! Lossless JPEG optimization, equivalent to `jpegtran -optimize -progressive -copy all`.
//!
//! The DCT coefficients are copied unchanged and only the entropy coding is
//! redone (optimized Huffman tables, progressive scans), so pixels stay
//! identical. All APPn/COM markers (EXIF, ICC profile, ...) are preserved.

use std::ffi::{c_int, c_uint, c_ulong, c_void};
use std::mem;
use std::panic::{self, AssertUnwindSafe};
use std::ptr;

use mozjpeg_sys::{
    jpeg_common_struct, jpeg_compress_struct, jpeg_copy_critical_parameters, jpeg_create_compress,
    jpeg_create_decompress, jpeg_decompress_struct, jpeg_destroy_compress, jpeg_destroy_decompress, jpeg_error_mgr,
    jpeg_finish_compress, jpeg_finish_decompress, jpeg_mem_dest, jpeg_mem_src, jpeg_read_coefficients,
    jpeg_read_header, jpeg_save_markers, jpeg_simple_progression, jpeg_std_error, jpeg_write_coefficients,
    jpeg_write_marker,
};

const JPEG_APP0: c_int = 0xE0;
const JPEG_COM: c_int = 0xFE;
const MAX_MARKER_LENGTH: c_uint = 0xFFFF;

unsafe extern "C" {
    // `jpeg_mem_dest` allocates its output buffer with malloc.
    fn free(ptr: *mut c_void);
}

/// Losslessly re-encodes a JPEG file.
pub fn optimize(input: &[u8]) -> Result<Vec<u8>, String> {
    // libjpeg reports errors through `error_exit`, which unwinds back to here.
    panic::catch_unwind(AssertUnwindSafe(|| unsafe { transcode(input) })).map_err(|payload| {
        let detail = payload.downcast_ref::<String>().map_or("unknown error", String::as_str);
        format!("Couldn't optimize the JPEG: {detail}")
    })
}

unsafe fn transcode(input: &[u8]) -> Vec<u8> {
    // Declaration order matters: guards are dropped in reverse, so the error
    // manager outlives both libjpeg objects that point to it.
    let mut error_mgr = new_error_mgr();
    let mut source = Decompress(Box::new(unsafe { mem::zeroed() }));
    let mut dest = Compress(Box::new(unsafe { mem::zeroed() }));
    let mut output = OutputBuffer { data: ptr::null_mut(), len: 0 };

    unsafe {
        let src = &mut *source.0;
        src.common.err = &mut *error_mgr;
        jpeg_create_decompress(src);
        jpeg_mem_src(src, input.as_ptr(), input.len() as c_ulong);
        jpeg_save_markers(src, JPEG_COM, MAX_MARKER_LENGTH);
        for app in 0..16 {
            jpeg_save_markers(src, JPEG_APP0 + app, MAX_MARKER_LENGTH);
        }
        jpeg_read_header(src, 1);
        let coefficients = jpeg_read_coefficients(src);

        let dst = &mut *dest.0;
        dst.common.err = &mut *error_mgr;
        jpeg_create_compress(dst);
        jpeg_copy_critical_parameters(src, dst);
        dst.optimize_coding = 1;
        jpeg_simple_progression(dst);
        jpeg_mem_dest(dst, &mut output.data, &mut output.len);
        jpeg_write_coefficients(dst, coefficients);
        copy_markers(src, dst);
        jpeg_finish_compress(dst);
        jpeg_finish_decompress(src);

        std::slice::from_raw_parts(output.data, output.len as usize).to_vec()
    }
}

/// Writes the saved markers to the output, skipping the JFIF and Adobe
/// headers that libjpeg already writes itself (same rule as jpegtran).
unsafe fn copy_markers(src: &jpeg_decompress_struct, dst: &mut jpeg_compress_struct) {
    let mut marker = src.marker_list;
    while let Some(current) = unsafe { marker.as_ref() } {
        let data = unsafe { std::slice::from_raw_parts(current.data, current.data_length as usize) };
        let code = c_int::from(current.marker);
        let is_jfif = code == JPEG_APP0 && data.starts_with(b"JFIF\0");
        let is_adobe = code == JPEG_APP0 + 14 && data.starts_with(b"Adobe");
        let written_by_libjpeg = (is_jfif && dst.write_JFIF_header != 0) || (is_adobe && dst.write_Adobe_marker != 0);
        if !written_by_libjpeg {
            unsafe { jpeg_write_marker(dst, code, current.data, current.data_length) };
        }
        marker = current.next;
    }
}

fn new_error_mgr() -> Box<jpeg_error_mgr> {
    let mut error_mgr: Box<jpeg_error_mgr> = Box::new(unsafe { mem::zeroed() });
    unsafe { jpeg_std_error(&mut error_mgr) };
    error_mgr.error_exit = Some(error_exit);
    error_mgr.emit_message = Some(emit_message);
    error_mgr
}

extern "C-unwind" fn error_exit(cinfo: &mut jpeg_common_struct) {
    let code = unsafe { cinfo.err.as_ref() }.map_or(0, |err| err.msg_code);
    // `resume_unwind` skips the panic hook, so nothing is printed.
    panic::resume_unwind(Box::new(format!("invalid JPEG data (libjpeg error {code})")));
}

/// Level -1 is a warning about corrupt or truncated data. libjpeg would carry
/// on and fill in the missing parts; treat it as an error so a damaged
/// original is never replaced. Other levels are trace messages.
extern "C-unwind" fn emit_message(cinfo: &mut jpeg_common_struct, level: c_int) {
    if level < 0 {
        let code = unsafe { cinfo.err.as_ref() }.map_or(0, |err| err.msg_code);
        panic::resume_unwind(Box::new(format!("corrupt or incomplete data (libjpeg warning {code})")));
    }
}

struct Decompress(Box<jpeg_decompress_struct>);

impl Drop for Decompress {
    fn drop(&mut self) {
        // Safe on a zeroed struct as well: libjpeg checks for a null memory manager.
        unsafe { jpeg_destroy_decompress(&mut self.0) };
    }
}

struct Compress(Box<jpeg_compress_struct>);

impl Drop for Compress {
    fn drop(&mut self) {
        unsafe { jpeg_destroy_compress(&mut self.0) };
    }
}

struct OutputBuffer {
    data: *mut u8,
    len: c_ulong,
}

impl Drop for OutputBuffer {
    fn drop(&mut self) {
        if !self.data.is_null() {
            unsafe { free(self.data.cast()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use image::codecs::jpeg::JpegEncoder;
    use image::{ImageDecoder, ImageEncoder, RgbImage};

    use super::*;

    const FAKE_ICC: &[u8] = b"fake icc profile used to check that markers are copied";

    fn sample_jpeg() -> Vec<u8> {
        let image =
            RgbImage::from_fn(128, 96, |x, y| image::Rgb([(x * 2) as u8, (y * 2) as u8, ((x * y) % 256) as u8]));
        let mut data = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut data, 90);
        encoder.set_icc_profile(FAKE_ICC.to_vec()).unwrap();
        encoder.write_image(image.as_raw(), 128, 96, image::ExtendedColorType::Rgb8).unwrap();
        data
    }

    #[test]
    fn keeps_pixels_and_metadata_and_shrinks() {
        let input = sample_jpeg();
        let output = optimize(&input).unwrap();

        assert!(output.len() < input.len(), "{} >= {}", output.len(), input.len());
        let before = image::load_from_memory(&input).unwrap().to_rgb8();
        let after = image::load_from_memory(&output).unwrap().to_rgb8();
        assert_eq!(before, after);

        let mut decoder = image::codecs::jpeg::JpegDecoder::new(Cursor::new(&output)).unwrap();
        assert_eq!(decoder.icc_profile().unwrap().as_deref(), Some(FAKE_ICC));
    }

    #[test]
    fn reports_invalid_data() {
        assert!(optimize(b"not a jpeg").is_err());
        let truncated = &sample_jpeg()[..200];
        assert!(optimize(truncated).is_err());
    }
}
