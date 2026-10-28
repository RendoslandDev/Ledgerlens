//! Minimal binary PGM (P5) / PPM (P6) codec. Handy for fixtures and debugging dumps.

use crate::image::{Image, MAX_IMAGE_BYTES};
use ll_core::{Error, Result};

pub(crate) const MAX_PNM_FILE_BYTES: u64 = MAX_IMAGE_BYTES as u64 + 1_048_576;

pub fn encode(img: &Image) -> Vec<u8> {
    let magic = if img.channels == 1 { "P5" } else { "P6" };
    let mut out = format!("{magic}\n{} {}\n255\n", img.width, img.height).into_bytes();
    out.extend_from_slice(&img.data);
    out
}

pub fn decode(bytes: &[u8]) -> Result<Image> {
    let bad = |m: &str| Error::parse(format!("pnm: {m}"));
    let mut pos = 0usize;
    let token = |pos: &mut usize| -> Result<String> {
        while *pos < bytes.len() {
            if bytes[*pos] == b'#' { 
                while *pos < bytes.len() && bytes[*pos] != b'\n' { *pos += 1; }
             }
            else if bytes[*pos].is_ascii_whitespace() {
                 *pos += 1; 
                } else { 
                    break; 
                }
        }
        let start = *pos;
        while *pos < bytes.len() && !bytes[*pos].is_ascii_whitespace() 
         { 
            *pos += 1; 
           }
        if start == *pos { 
            return Err(bad("unexpected end of header")); 
        }
        Ok(String::from_utf8_lossy(&bytes[start..*pos]).into_owned())
    };
    let channels = match token(&mut pos)?.as_str() 
        {
             "P5" => 1, "P6" => 3, m => return Err(bad(&format!("unsupported magic {m}")))
             };
    let w: usize = token(&mut pos)?.parse().map_err(|_| bad("bad width"))?;
    let h: usize = token(&mut pos)?.parse().map_err(|_| bad("bad height"))?;
    let maxval: usize = token(&mut pos)?.parse().map_err(|_| bad("bad maxval"))?;
    if maxval != 255 {
         return Err(bad("only maxval 255 supported")); 
        }
    if pos >= bytes.len() || !bytes[pos].is_ascii_whitespace() {
         return Err(bad("missing header separator")); 
        }
    if bytes[pos] == b'\r' && bytes.get(pos + 1) == Some(&b'\n') { pos += 2; } else { pos += 1; }
    let need = Image::checked_data_len(w, h, channels).map_err(|_| bad("image dimensions exceed limits"))?;
    let end = pos.checked_add(need).ok_or_else(|| bad("pixel data length overflows"))?;
    if bytes.len() < end { 
        return Err(bad("truncated pixel data"));
         }
    Image::new(w, h, channels, bytes[pos..end].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_gray_and_rgb() {
        let g = Image::new(3, 2, 1, vec![1, 2, 3, 4, 5, 6]).unwrap();
        assert_eq!(decode(&encode(&g)).unwrap(), g);
        let c = Image::new(1, 2, 3, vec![1, 2, 3, 4, 5, 6]).unwrap();
        assert_eq!(decode(&encode(&c)).unwrap(), c);
    }
    #[test]
    fn handles_comments_and_errors() {
        let mut b = b"P5\n# hi\n2 1\n255\n".to_vec();
        b.extend([7, 8]);
        assert_eq!(decode(&b).unwrap().data, vec![7, 8]);
        assert!(decode(b"P3\n1 1\n255\n0").is_err());
        assert!(decode(b"P5\n2 2\n255\n\x00").is_err());
    }

    #[test]
    fn rejects_dimension_overflow_and_oversized_images_before_allocating() {
        assert!(decode(b"P5\n18446744073709551615 2\n255\n").is_err());
        assert!(decode(b"P5\n16001 1001\n255\n").is_err());
    }

    #[test]
    fn accepts_crlf_header_separator_without_shifting_pixel_data() {
        assert_eq!(decode(b"P5\r\n1 1\r\n255\r\n\x07").unwrap().data(), &[7]);
    }
}
