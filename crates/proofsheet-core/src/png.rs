//! Just enough PNG to verify what the browser handed back.
//!
//! We never decode pixels; we only read the header, because the one thing
//! worth asserting is that the image is exactly the size that was requested.

use crate::error::{Error, Result};

const MAGIC: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

/// Width and height straight out of the IHDR chunk.
pub fn dimensions(data: &[u8]) -> Result<(u32, u32)> {
    if data.len() < 24 {
        return Err(Error::Shape(format!("png too short: {} bytes", data.len())));
    }
    if data[..8] != MAGIC {
        return Err(Error::Shape("not a png (bad magic)".into()));
    }
    if &data[12..16] != b"IHDR" {
        return Err(Error::Shape("first chunk is not IHDR".into()));
    }
    let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    Ok((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(w: u32, h: u32) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&MAGIC);
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v
    }

    #[test]
    fn reads_dimensions() {
        assert_eq!(dimensions(&header(1290, 2796)).unwrap(), (1290, 2796));
    }

    #[test]
    fn rejects_non_png() {
        let mut bad = header(1, 1);
        bad[1] = b'X';
        assert!(dimensions(&bad).is_err());
    }

    #[test]
    fn rejects_truncated() {
        assert!(dimensions(&[0u8; 10]).is_err());
    }
}
