use ll_core::{Error, Result};

/// A dense row-major f32 tensor.
#[derive(Debug, Clone, PartialEq)]
pub struct Tensor {
    pub shape: Vec<usize>,
    pub data: Vec<f32>,
}

impl Tensor {
    pub fn new(shape: Vec<usize>, data: Vec<f32>) -> Result<Self> {
        let n: usize = shape.iter().product();
        if n != data.len() {
            return Err(Error::Inference(format!("shape {shape:?} needs {n} values, got {}", data.len())));
        }
        Ok(Tensor { shape, data })
    }
    pub fn zeros(shape: Vec<usize>) -> Self {
        let n = shape.iter().product();
        Tensor { shape, data: vec![0.0; n] }
    }
    pub fn numel(&self) -> usize { self.data.len() }

    /// Index of the max value along the last axis for each leading row.
    pub fn argmax_last(&self) -> Vec<usize> {
        let Some(&c) = self.shape.last() else { return vec![] };
        if c == 0 { return vec![]; }
        self.data
            .chunks(c)
            .map(|row| row.iter().enumerate().fold((0, f32::NEG_INFINITY), |b, (i, &v)| if v > b.1 { (i, v) } else { b }).0)
            .collect()
    }

    /// Softmax over the last axis.
    pub fn softmax_last(&self) -> Tensor {
        let c = self.shape.last().copied().unwrap_or(1).max(1);
        let mut out = self.data.clone();
        for row in out.chunks_mut(c) {
            let m = row.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
            let mut s = 0.0;
            for v in row.iter_mut() { *v = (*v - m).exp(); s += *v; }
            for v in row.iter_mut() { *v /= s; }
        }
        Tensor { shape: self.shape.clone(), data: out }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shape_checked() { assert!(Tensor::new(vec![2, 2], vec![1.0; 3]).is_err()); }
    #[test]
    fn argmax_and_softmax() {
        let t = Tensor::new(vec![2, 3], vec![0.1, 0.9, 0.0, 5.0, 1.0, 2.0]).unwrap();
        assert_eq!(t.argmax_last(), vec![1, 0]);
        let s = t.softmax_last();
        assert!((s.data[0..3].iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }
}
