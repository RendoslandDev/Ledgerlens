use ll_core::{Error, Result};

pub(crate) const MAX_IMAGE_PIXELS: usize = 16_000_000;
pub(crate) const MAX_IMAGE_BYTES: usize = MAX_IMAGE_PIXELS * 3;

/// 8-bit interleaved image with 1 (gray) or 3 (RGB) channels.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub(crate) width: usize,
    pub(crate) height: usize,
    pub(crate) channels: usize,
    pub(crate) data: Vec<u8>,
}

impl Image {
    pub fn width(&self) -> usize {
         self.width
         }
    pub fn height(&self) -> usize {
         self.height 
        }
    pub fn channels(&self) -> usize {
         self.channels 
        }
    pub fn data(&self) -> &[u8] {
         &self.data 
        }

    pub(crate) fn checked_data_len(width: usize, height: usize, channels: usize) -> Result<usize> {
        if channels != 1 && channels != 3 { return Err(Error::validation("channels must be 1 or 3")); }
        let pixels = width.checked_mul(height)
            .filter(|pixels| *pixels <= MAX_IMAGE_PIXELS)
            .ok_or_else(|| Error::validation("image dimensions exceed the pixel limit"))?;
        pixels.checked_mul(channels)
            .ok_or_else(|| Error::validation("image byte length overflows"))
    }

    pub fn new(width: usize, height: usize, channels: usize, data: Vec<u8>) -> Result<Self> {
        let expected = Self::checked_data_len(width, height, channels)?;
        if data.len() != expected {
            return Err(Error::validation(format!("expected {} bytes, got {}", expected, data.len())));
        }
        Ok(Image { width, height, channels, data })
    }

    pub fn filled(width: usize, height: usize, channels: usize, v: u8) -> Result<Self> {
        let size = Self::checked_data_len(width, height, channels)?;
        Ok(Image { width, height, channels, data: vec![v; size] })
    }

    #[inline]
    pub fn px(&self, x: usize, y: usize, c: usize) -> u8 { self.data[(y * self.width + x) * self.channels + c] }

    pub fn to_gray(&self) -> Image {
        if self.channels == 1 { return self.clone(); }
        let mut out = Vec::with_capacity(self.width * self.height);
        for p in self.data.chunks_exact(3) {
            // ITU-R BT.601 luma, integer math
            out.push(((299 * p[0] as u32 + 587 * p[1] as u32 + 114 * p[2] as u32) / 1000) as u8);
        }
        Image { width: self.width, height: self.height, channels: 1, data: out }
    }

    /// Bilinear resize.
    pub fn resize(&self, nw: usize, nh: usize) -> Result<Image> {
        if nw == 0 || nh == 0 || self.width == 0 || self.height == 0 { return Err(Error::validation("zero-sized image")); }
        let capacity = Self::checked_data_len(nw, nh, self.channels)?;
        let mut out = Vec::with_capacity(capacity);
        let sx = self.width as f32 / nw as f32;
        let sy = self.height as f32 / nh as f32;
        for y in 0..nh {
            let fy = ((y as f32 + 0.5) * sy - 0.5).clamp(0.0, (self.height - 1) as f32);
            let (y0, y1) = (fy.floor() as usize, (fy.ceil() as usize).min(self.height - 1));
            let wy = fy - y0 as f32;
            for x in 0..nw {
                let fx = ((x as f32 + 0.5) * sx - 0.5).clamp(0.0, (self.width - 1) as f32);
                let (x0, x1) = (fx.floor() as usize, (fx.ceil() as usize).min(self.width - 1));
                let wx = fx - x0 as f32;
                for c in 0..self.channels {
                    let top = self.px(x0, y0, c) as f32 * (1.0 - wx) + self.px(x1, y0, c) as f32 * wx;
                    let bot = self.px(x0, y1, c) as f32 * (1.0 - wx) + self.px(x1, y1, c) as f32 * wx;
                    out.push((top * (1.0 - wy) + bot * wy).round().clamp(0.0, 255.0) as u8);
                }
            }
        }
        Ok(Image { width: nw, height: nh, channels: self.channels, data: out })
    }

    /// Scales so the longer side equals `max_side` (never upscales unless `allow_upscale`).
    pub fn fit_max_side(&self, max_side: usize, allow_upscale: bool) -> Result<Image> {
        let longest = self.width.max(self.height);
        if longest == 0 { return Err(Error::validation("zero-sized image")); }
        if longest <= max_side && !allow_upscale { return Ok(self.clone()); }
        let s = max_side as f32 / longest as f32;
        self.resize(((self.width as f32 * s).round() as usize).max(1), ((self.height as f32 * s).round() as usize).max(1))
    }

    pub fn crop(&self, x: usize, y: usize, w: usize, h: usize) -> Result<Image> {
        if w == 0 || h == 0
            || x.checked_add(w).is_none_or(|right| right > self.width)
            || y.checked_add(h).is_none_or(|bottom| bottom > self.height)
        { return Err(Error::validation("crop out of bounds")); }
        let capacity = Self::checked_data_len(w, h, self.channels)?;
        let mut out = Vec::with_capacity(capacity);
        for row in y..y + h {
            let start = (row * self.width + x) * self.channels;
            out.extend_from_slice(&self.data[start..start + w * self.channels]);
        }
        Ok(Image { width: w, height: h, channels: self.channels, data: out })
    }

    /// Stretches the histogram so min→0 and max→255.
    pub fn autocontrast(&self) -> Image {
        let (lo, hi) = self.data.iter().fold((255u8, 0u8), |(l, h), &v| (l.min(v), h.max(v)));
        if hi <= lo { return self.clone(); }
        let range = (hi - lo) as f32;
        Image { data: self.data.iter().map(|&v| (((v - lo) as f32 / range) * 255.0).round() as u8).collect(), ..self.clone() }
    }

    /// Otsu's threshold on a gray image (returns 128 for degenerate input).
    pub fn otsu_threshold(&self) -> u8 {
        let g = self.to_gray();
        let mut hist = [0u64; 256];
        for &v in &g.data { hist[v as usize] += 1; }
        let total = g.data.len() as f64;
        if total == 0.0 { return 128; }
        let sum_all: f64 = hist.iter().enumerate().map(|(i, &c)| i as f64 * c as f64).sum();
        let (mut sum_b, mut w_b, mut best, mut thr) = (0.0f64, 0.0f64, -1.0f64, 128u8);
        for (t, &count) in hist.iter().enumerate() {
            w_b += count as f64;
            if w_b == 0.0 { continue; }
            let w_f = total - w_b;
            if w_f == 0.0 { break; }
            sum_b += t as f64 * count as f64;
            let (m_b, m_f) = (sum_b / w_b, (sum_all - sum_b) / w_f);
            let between = w_b * w_f * (m_b - m_f) * (m_b - m_f);
            if between > best { best = between; thr = t as u8; }
        }
        thr
    }

    /// Black/white image: pixels `> thr` become 255, others 0.
    pub fn binarize(&self, thr: u8) -> Image {
        let g = self.to_gray();
        Image { data: g.data.iter().map(|&v| if v > thr { 255 } else { 0 }).collect(), ..g }
    }

    pub fn invert(&self) -> Image { Image { data: self.data.iter().map(|&v| 255 - v).collect(), ..self.clone() } }

    /// Pads with `fill` on the right/bottom so both dimensions are multiples of `m`.
    pub fn pad_to_multiple(&self, m: usize, fill: u8) -> Result<Image> {
        let m = m.max(1);
        let nw = self.width.div_ceil(m).checked_mul(m)
            .ok_or_else(|| Error::validation("padded image width overflows"))?;
        let nh = self.height.div_ceil(m).checked_mul(m)
            .ok_or_else(|| Error::validation("padded image height overflows"))?;
        if (nw, nh) == (self.width, self.height) { return Ok(self.clone()); }
        let size = Self::checked_data_len(nw, nh, self.channels)?;
        let mut out = vec![fill; size];
        for y in 0..self.height {
            let src = y * self.width * self.channels;
            let dst = y * nw * self.channels;
            out[dst..dst + self.width * self.channels].copy_from_slice(&self.data[src..src + self.width * self.channels]);
        }
        Ok(Image { width: nw, height: nh, channels: self.channels, data: out })
    }

    /// Fraction of "ink" (dark) pixels — handy for blank-page detection.
    pub fn ink_ratio(&self, dark_below: u8) -> f32 {
        let g = self.to_gray();
        if g.data.is_empty() { return 0.0; }
        g.data.iter().filter(|&&v| v < dark_below).count() as f32 / g.data.len() as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: usize, h: usize) -> Image {
        Image::new(w, h, 1, (0..w * h).map(|i| ((i % w) * 255 / (w - 1).max(1)) as u8).collect()).unwrap()
    }

    #[test]
    fn validates_size() { assert!(Image::new(2, 2, 1, vec![0; 3]).is_err()); assert!(Image::new(1, 1, 2, vec![0; 2]).is_err()); }

    #[test]
    fn gray_conversion() {
        let rgb = Image::new(2, 1, 3, vec![255, 0, 0, 255, 255, 255]).unwrap();
        let g = rgb.to_gray();
        assert_eq!(g.data, vec![76, 255]);
    }

    #[test]
    fn resize_identity_and_shrink() {
        let im = gradient(8, 4);
        assert_eq!(im.resize(8, 4).unwrap(), im);
        let small = im.resize(4, 2).unwrap();
        assert_eq!((small.width, small.height), (4, 2));
        assert!(small.data[0] < small.data[3]);
    }

    #[test]
    fn fit_max_side() {
        let im = Image::filled(400, 200, 1, 10).unwrap();
        let f = im.fit_max_side(100, false).unwrap();
        assert_eq!((f.width, f.height), (100, 50));
        assert_eq!(im.fit_max_side(1000, false).unwrap().width, 400);
        assert_eq!(im.fit_max_side(800, true).unwrap().width, 800);
    }

    #[test]
    fn crop_and_bounds() {
        let im = gradient(10, 10);
        let c = im.crop(2, 3, 4, 5).unwrap();
        assert_eq!((c.width, c.height), (4, 5));
        assert_eq!(c.px(0, 0, 0), im.px(2, 3, 0));
        assert!(im.crop(8, 8, 5, 5).is_err());
    }

    #[test]
    fn otsu_separates_two_modes() {
        let mut d = vec![30u8; 50];
        d.extend(vec![220u8; 50]);
        let im = Image::new(10, 10, 1, d).unwrap();
        let t = im.otsu_threshold();
        assert!((30..220).contains(&t), "threshold {t}");
        let b = im.binarize(t);
        assert_eq!(b.data.iter().filter(|&&v| v == 255).count(), 50);
    }

    #[test]
    fn autocontrast_and_invert() {
        let im = Image::new(3, 1, 1, vec![100, 150, 200]).unwrap();
        assert_eq!(im.autocontrast().data, vec![0, 128, 255]);
        assert_eq!(im.invert().data, vec![155, 105, 55]);
    }

    #[test]
    fn padding() {
        let p = Image::filled(5, 3, 1, 1).unwrap().pad_to_multiple(4, 9).unwrap();
        assert_eq!((p.width, p.height), (8, 4));
        assert_eq!(p.px(7, 3, 0), 9);
        assert_eq!(p.px(4, 2, 0), 1);
    }

    #[test]
    fn ink_ratio_detects_blank() {
        assert_eq!(Image::filled(10, 10, 1, 255).unwrap().ink_ratio(128), 0.0);
        assert_eq!(Image::filled(10, 10, 1, 0).unwrap().ink_ratio(128), 1.0);
    }

    #[test]
    fn rejects_overflowing_or_oversized_dimensions() {
        assert!(Image::new(usize::MAX, 2, 1, Vec::new()).is_err());
        assert!(Image::filled(usize::MAX, 2, 1, 0).is_err());
        let image = Image::filled(1, 1, 1, 0).unwrap();
        assert!(image.resize(usize::MAX, 1).is_err());
        assert!(image.crop(usize::MAX, 0, 1, 1).is_err());
        assert!(Image::filled(1, 1, 1, 0).unwrap().pad_to_multiple(usize::MAX, 0).is_err());
    }
}
