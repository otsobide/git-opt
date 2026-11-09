//! JPEG via mozjpeg, built in.
//!
//! This is a lossless transcode of the DCT coefficients, the same operation as
//! `jpegtran -copy all -optimize -progressive`, done with mozjpeg's library:
//! optimized Huffman tables plus mozjpeg's progressive scan search. The
//! coefficients (and therefore the decoded pixels) never change, and every
//! APPn/COM segment (EXIF, orientation, ICC profile, XMP, comments) is copied.
//!
//! libjpeg reports errors through callbacks that normally `exit()`. Here they
//! unwind back into Rust (`C-unwind`, mozjpeg built with `-fexceptions`), so
//! a broken file becomes an `Err`. Warnings (corrupt data) are treated as
//! errors too: we never rewrite a file the decoder had to guess about.

use std::{
    ffi::c_void,
    mem,
    os::raw::{c_int, c_ulong},
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    ptr,
    sync::Arc,
};

use anyhow::{Result, anyhow, ensure};
use async_trait::async_trait;
use image::ImageFormat;
use mozjpeg_sys::{
    J_BOOLEAN_PARAM, JCOPY_OPTION_JCOPYOPT_ALL, JPEG_LIB_VERSION, jcopy_markers_execute, jcopy_markers_setup,
    jpeg_CreateCompress, jpeg_CreateDecompress, jpeg_c_set_bool_param, jpeg_common_struct, jpeg_compress_struct,
    jpeg_copy_critical_parameters, jpeg_decompress_struct, jpeg_destroy_compress, jpeg_destroy_decompress,
    jpeg_error_mgr, jpeg_finish_compress, jpeg_mem_dest, jpeg_mem_src, jpeg_read_coefficients, jpeg_read_header,
    jpeg_std_error, jpeg_write_coefficients,
};

use git_opt_domain::{
    model::CodecOptions,
    ports::FormatStrategy,
    values::{FileFormat, Profile},
};

use crate::verify;

pub struct Mozjpeg;

#[async_trait]
impl FormatStrategy for Mozjpeg {
    fn id(&self) -> &'static str {
        "mozjpeg"
    }
    fn label(&self) -> &'static str {
        "JPEG (mozjpeg)"
    }
    fn formats(&self) -> &'static [FileFormat] {
        &[FileFormat::Jpeg]
    }
    fn explanation(&self) -> &'static str {
        "**JPEG**: lossless transcode with [mozjpeg](https://github.com/mozilla/mozjpeg) (like `jpegtran -copy all -optimize -progressive`): optimized Huffman tables and scans; DCT coefficients and all metadata (EXIF, ICC, XMP) are untouched."
    }
    fn profile(&self, _: &CodecOptions) -> Profile {
        Profile::new("mozjpeg4:copyall:best-of-progressive-baseline")
    }

    async fn probe(&self) -> std::result::Result<String, String> {
        Ok("built-in mozjpeg 4".into())
    }

    async fn optimize(&self, input: Arc<Vec<u8>>, _: &CodecOptions) -> Result<Vec<u8>> {
        tokio::task::spawn_blocking(move || {
            let progressive = transcode(&input, true)?;
            let baseline = transcode(&input, false)?;
            // Progressive usually wins; tiny images are often smaller as baseline.
            Ok(if baseline.len() < progressive.len() { baseline } else { progressive })
        })
        .await?
    }

    async fn verify(&self, original: Arc<Vec<u8>>, candidate: Arc<Vec<u8>>) -> Result<()> {
        tokio::task::spawn_blocking(move || {
            same_metadata(&original, &candidate)?;
            verify::same_pixels(&original, &candidate, ImageFormat::Jpeg)
        })
        .await?
    }
}

// ---------------------------------------------------------------- transcode

/// Losslessly re-encodes a JPEG. `progressive` selects mozjpeg's optimized
/// progressive scans; otherwise a baseline (sequential) file is written.
pub fn transcode(input: &[u8], progressive: bool) -> Result<Vec<u8>> {
    match catch_unwind(AssertUnwindSafe(|| unsafe { transcode_unchecked(input, progressive) })) {
        Ok(result) => result,
        Err(payload) => Err(anyhow!(
            "{}",
            payload
                .downcast_ref::<String>()
                .cloned()
                .unwrap_or_else(|| "libjpeg failed".into())
        )),
    }
}

unsafe extern "C" {
    fn free(ptr: *mut c_void);
}

/// Owns every libjpeg object so they are released even when an error unwinds.
/// Everything is boxed: libjpeg keeps pointers to these structs.
struct Session {
    src: Box<jpeg_decompress_struct>,
    dst: Box<jpeg_compress_struct>,
    _src_err: Box<jpeg_error_mgr>,
    _dst_err: Box<jpeg_error_mgr>,
    out_buf: Box<*mut u8>,
    out_size: Box<c_ulong>,
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            jpeg_destroy_compress(&mut self.dst);
            jpeg_destroy_decompress(&mut self.src);
            if !self.out_buf.is_null() {
                free(*self.out_buf as *mut c_void);
            }
        }
    }
}

unsafe fn error_manager() -> Box<jpeg_error_mgr> {
    unsafe {
        let mut err: Box<jpeg_error_mgr> = Box::new(mem::zeroed());
        jpeg_std_error(&mut err);
        err.error_exit = Some(error_exit);
        err.emit_message = Some(emit_message);
        err
    }
}

unsafe fn transcode_unchecked(input: &[u8], progressive: bool) -> Result<Vec<u8>> {
    unsafe {
        let mut s = Session {
            src: Box::new(mem::zeroed()),
            dst: Box::new(mem::zeroed()),
            _src_err: error_manager(),
            _dst_err: error_manager(),
            out_buf: Box::new(ptr::null_mut()),
            out_size: Box::new(0),
        };
        s.src.common.err = &mut *s._src_err;
        s.dst.common.err = &mut *s._dst_err;
        jpeg_CreateDecompress(&mut *s.src, JPEG_LIB_VERSION, mem::size_of::<jpeg_decompress_struct>());
        jpeg_CreateCompress(&mut *s.dst, JPEG_LIB_VERSION, mem::size_of::<jpeg_compress_struct>());

        jpeg_mem_src(&mut s.src, input.as_ptr(), input.len() as c_ulong);
        // Keep every APPn and COM segment (EXIF, ICC, XMP, comments…).
        jcopy_markers_setup(&mut *s.src, JCOPY_OPTION_JCOPYOPT_ALL);
        jpeg_read_header(&mut s.src, 1);
        let coefficients = jpeg_read_coefficients(&mut s.src);
        ensure!(!coefficients.is_null(), "could not read DCT coefficients");

        // With mozjpeg's default (max compression) profile this also turns on
        // Huffman optimization, scan optimization and progressive mode.
        jpeg_copy_critical_parameters(&s.src, &mut s.dst);
        s.dst.optimize_coding = 1;
        if !progressive {
            jpeg_c_set_bool_param(&mut s.dst, J_BOOLEAN_PARAM::JBOOLEAN_OPTIMIZE_SCANS, 0);
            s.dst.scan_info = ptr::null();
            s.dst.num_scans = 0;
        }

        jpeg_mem_dest(&mut s.dst, &mut *s.out_buf, &mut *s.out_size);
        jpeg_write_coefficients(&mut s.dst, coefficients);
        jcopy_markers_execute(&mut *s.src, &mut *s.dst, JCOPY_OPTION_JCOPYOPT_ALL);
        jpeg_finish_compress(&mut s.dst);

        ensure!(!s.out_buf.is_null() && *s.out_size > 0, "mozjpeg produced no output");
        Ok(std::slice::from_raw_parts(*s.out_buf, *s.out_size as usize).to_vec())
    }
}

fn message(cinfo: &mut jpeg_common_struct) -> String {
    unsafe {
        let err = &*cinfo.err;
        match err.format_message {
            Some(format) => {
                let mut buffer = [0u8; 80];
                // The binding declares the buffer as `&[u8; 80]` but libjpeg writes into it.
                let format: unsafe extern "C-unwind" fn(&mut jpeg_common_struct, &mut [u8; 80]) =
                    mem::transmute(format);
                format(cinfo, &mut buffer);
                let end = buffer.iter().position(|&b| b == 0).unwrap_or(buffer.len());
                String::from_utf8_lossy(&buffer[..end]).into_owned()
            }
            None => format!("libjpeg error {}", err.msg_code),
        }
    }
}

extern "C-unwind" fn error_exit(cinfo: &mut jpeg_common_struct) {
    // `resume_unwind` skips the panic hook: no noise on stderr.
    resume_unwind(Box::new(message(cinfo)));
}

extern "C-unwind" fn emit_message(cinfo: &mut jpeg_common_struct, level: c_int) {
    if level < 0 {
        resume_unwind(Box::new(format!("refusing a damaged JPEG: {}", message(cinfo))));
    }
}

// ----------------------------------------------------------------- metadata

/// APPn and COM segments before the first scan, in order.
pub(super) fn metadata_segments(data: &[u8]) -> Result<Vec<(u8, &[u8])>> {
    ensure!(data.starts_with(&[0xFF, 0xD8]), "not a JPEG");
    let mut segments = Vec::new();
    let mut pos = 2;
    loop {
        ensure!(pos < data.len() && data[pos] == 0xFF, "malformed JPEG segment at byte {pos}");
        while pos < data.len() && data[pos] == 0xFF {
            pos += 1; // fill bytes
        }
        let marker = *data.get(pos).ok_or_else(|| anyhow!("truncated JPEG"))?;
        pos += 1;
        match marker {
            0xDA | 0xD9 => return Ok(segments), // start of scan / end of image
            0x01 | 0xD0..=0xD7 => continue,     // markers without a length
            _ => {}
        }
        let len = u16::from_be_bytes([data.get(pos).copied().unwrap_or(0), data.get(pos + 1).copied().unwrap_or(0)]) as usize;
        ensure!(len >= 2 && pos + len <= data.len(), "truncated JPEG segment");
        if matches!(marker, 0xE0..=0xEF | 0xFE) {
            segments.push((marker, &data[pos + 2..pos + len]));
        }
        pos += len;
    }
}

fn is_jfif(segment: &(u8, &[u8])) -> bool {
    segment.0 == 0xE0 && segment.1.starts_with(b"JFIF\0")
}

fn is_adobe(segment: &(u8, &[u8])) -> bool {
    segment.0 == 0xEE && segment.1.starts_with(b"Adobe")
}

/// Every APPn/COM segment survives byte for byte and in order. The JFIF
/// header is compared by content, not position: the encoder always writes it
/// first (as the standard requires) and may add one when there was none. The
/// Adobe color-transform marker is rewritten by the encoder; its effect on
/// colors is covered by the pixel comparison.
fn same_metadata(original: &[u8], candidate: &[u8]) -> Result<()> {
    let before = metadata_segments(original)?;
    let after = metadata_segments(candidate)?;
    ensure!(
        before.iter().any(is_adobe) == after.iter().any(is_adobe),
        "Adobe color transform marker changed"
    );
    if let Some(jfif) = before.iter().find(|s| is_jfif(s)) {
        ensure!(after.iter().find(|s| is_jfif(s)) == Some(jfif), "JFIF header changed");
    }
    let other = |segments: Vec<(u8, &'_ [u8])>| -> Vec<(u8, Vec<u8>)> {
        segments
            .into_iter()
            .filter(|s| !is_adobe(s) && !is_jfif(s))
            .map(|(m, p)| (m, p.to_vec()))
            .collect()
    };
    ensure!(other(before) == other(after), "JPEG metadata (APPn/COM segments) changed");
    Ok(())
}
