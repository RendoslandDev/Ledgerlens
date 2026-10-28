use crate::image::Image;
use ll_core::Result;
use ll_infer::Tensor;

/// Configurable preprocessing pipeline producing model-ready images.
#[derive(Debug, Clone)]
pub struct Preprocess {
    pub max_side: usize,
    pub grayscale: bool,
    pub autocontrast: bool,
    /// When `Some`, binarises with the given threshold, or Otsu when `Some(None)`.
    pub binarize: Option<Option<u8>>,
    pub pad_multiple: usize,
}

impl Default for Preprocess {
    fn default() -> Self {
        Preprocess { 
            max_side: 1600,
            grayscale: true, 
            autocontrast: true,
            binarize: None,
            pad_multiple: 32,
         }
    }
}

impl Preprocess {
    pub fn apply(&self, img: &Image) -> Result<Image> {
        let mut im = img.fit_max_side(self.max_side, false)?;
        if self.grayscale {
             im = im.to_gray();
             }
        if self.autocontrast {
             im = im.autocontrast();
             }
        if let Some(thr) = self.binarize {
            let t = thr.unwrap_or_else(|| im.otsu_threshold());
            im = im.binarize(t);
        }
        im.pad_to_multiple(self.pad_multiple.max(1), 255)
    }
}

/// Converts an image to a `[C, H, W]` f32 tensor: `(v/255 - mean[c]) / std[c]`.
pub fn to_tensor_chw(img: &Image, mean: &[f32], std: &[f32]) -> Result<Tensor> {
    let (w, h, c) = (img.width, img.height, img.channels);
    let mut data = vec![0.0f32; c * h * w];
    for ch in 0..c {
        let (m, s) = (mean.get(ch).copied().unwrap_or(0.0), std.get(ch).copied().unwrap_or(1.0).max(1e-6));
        for y in 0..h {
            for x in 0..w {
                data[ch * h * w + y * w + x] = (img.px(x, y, ch) as f32 / 255.0 - m) / s;
            }
        }
    }
    Tensor::new(vec![c, h, w], data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_output_shape() {
        let im = Image::filled(100, 50, 3, 128).unwrap();
        let out = Preprocess { max_side: 64, pad_multiple: 32, ..Default::default() }.apply(&im).unwrap();
        assert_eq!(out.channels, 1);
        assert_eq!((out.width, out.height), (64, 32));
    }

    #[test]
    fn tensor_is_chw_and_normalised() {
        let im = Image::new(2, 1, 3, vec![255, 0, 0, 0, 255, 0]).unwrap();
        let t = to_tensor_chw(&im, &[0.5; 3], &[0.5; 3]).unwrap();
        assert_eq!(t.shape, vec![3, 1, 2]);
        assert_eq!(t.data, vec![1.0, -1.0, -1.0, 1.0, -1.0, -1.0]);
    }

    #[test]
    fn binarize_option() {
        let mut d = vec![10u8; 8];
        d.extend(vec![240u8; 8]);
        let im = Image::new(4, 4, 1, d).unwrap();
        let out = Preprocess { binarize: Some(None), autocontrast: false, pad_multiple: 1, ..Default::default() }.apply(&im).unwrap();
        assert!(out.data.iter().all(|&v| v == 0 || v == 255));
    }
}
