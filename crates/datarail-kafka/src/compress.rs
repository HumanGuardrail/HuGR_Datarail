//! Decompression of compressed Kafka producer batches (feature `compression`, `KAFKA-COMPRESSION-DESIGN.md`).
//!
//! Decompress-only, pure-Rust codecs. A compressed v2 `RecordBatch` has an uncompressed header followed by a
//! compressed records blob (codec = `attributes & 0x07`); we decompress the blob and parse the records from it, so
//! they become normal sealed cofres. Every decompression is **bounded** by `max` (a zip-bomb guard): a small
//! compressed batch from an untrusted producer cannot expand to exhaust memory.

use std::io;

/// Decompress a compressed records blob. `codec` is the Kafka attributes codec id (1 = gzip, 2 = snappy, 3 = lz4,
/// 4 = zstd). At most `max` decompressed bytes are produced; anything larger is rejected.
///
/// # Errors
/// [`io::Error`] if the codec is unsupported (not compiled in) or the stream is malformed / exceeds `max`.
pub fn decompress(codec: u8, input: &[u8], max: usize) -> io::Result<Vec<u8>> {
    match codec {
        #[cfg(feature = "compression-gzip")]
        1 => gunzip(input, max),
        #[cfg(feature = "compression-lz4")]
        3 => unlz4(input, max),
        #[cfg(feature = "compression-zstd")]
        4 => unzstd(input, max),
        other => {
            Err(io::Error::new(io::ErrorKind::InvalidData, format!("unsupported compression codec {other}")))
        }
    }
}

/// Read a decompressing reader to completion, bounded at `max` bytes (the shared zip-bomb guard).
#[cfg(any(feature = "compression-lz4", feature = "compression-zstd"))]
fn read_capped(mut r: impl std::io::Read, max: usize) -> io::Result<Vec<u8>> {
    use std::io::Read as _;
    let cap = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    let mut out = Vec::new();
    r.by_ref().take(cap).read_to_end(&mut out)?;
    if out.len() > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "decompressed batch exceeds the size cap"));
    }
    Ok(out)
}

/// lz4 (Kafka codec 3) — the LZ4 **frame** format (not raw block) via `lz4_flex`, capped at `max`.
#[cfg(feature = "compression-lz4")]
fn unlz4(input: &[u8], max: usize) -> io::Result<Vec<u8>> {
    read_capped(lz4_flex::frame::FrameDecoder::new(input), max)
}

/// zstd (Kafka codec 4) — a standard zstd frame via the pure-Rust `ruzstd` decoder, capped at `max`.
#[cfg(feature = "compression-zstd")]
fn unzstd(input: &[u8], max: usize) -> io::Result<Vec<u8>> {
    let dec = ruzstd::decoding::StreamingDecoder::new(input)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    read_capped(dec, max)
}

/// gzip (Kafka codec 1) — a standard gzip stream via flate2's pure-Rust backend, capped at `max` bytes.
#[cfg(feature = "compression-gzip")]
fn gunzip(input: &[u8], max: usize) -> io::Result<Vec<u8>> {
    use std::io::Read as _;
    // Read at most max+1 bytes so an over-cap stream is detected without materializing the whole thing.
    let cap = u64::try_from(max).unwrap_or(u64::MAX).saturating_add(1);
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(input).take(cap).read_to_end(&mut out)?;
    if out.len() > max {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "decompressed batch exceeds the size cap"));
    }
    Ok(out)
}
