use ll_core::{Error, Result};
use ll_infer::Tensor;

/// Greedy CTC decode of `[T, C]` (already softmaxed or raw logits) with blank = class 0.
/// `charset[i]` is the character for class `i + 1`. Returns text and mean per-character confidence
/// (computed on softmaxed probabilities).
pub fn ctc_greedy_decode(logits: &Tensor, charset: &[char]) -> Result<(String, f32)> {
    if logits.shape.len() != 2 { return Err(Error::Inference(format!("expected [T,C], got {:?}", logits.shape))); }
    let c = logits.shape[1];
    if c != charset.len() + 1 { return Err(Error::Inference(format!("{c} classes but charset has {} (+blank)", charset.len()))); }
    let probs = logits.softmax_last();
    let best = logits.argmax_last();

    let (mut text, mut conf_sum, mut n) = (String::new(), 0.0f32, 0usize);
    let mut prev = 0usize;
    for (t, &k) in best.iter().enumerate() {
        if k != 0 && k != prev {
            text.push(charset[k - 1]);
            conf_sum += probs.data[t * c + k];
            n += 1;
        }
        prev = k;
    }
    Ok((text, if n == 0 { 0.0 } else { conf_sum / n as f32 }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn onehot(seq: &[usize], classes: usize) -> Tensor {
        let mut d = vec![0.0f32; seq.len() * classes];
        for (t, &k) in seq.iter().enumerate() { d[t * classes + k] = 10.0; }
        Tensor::new(vec![seq.len(), classes], d).unwrap()
    }

    #[test]
    fn collapses_repeats_and_blanks() {
        let cs = ['a', 'b', 'c'];
        // blank a a blank a b b -> "aab"
        let (t, conf) = ctc_greedy_decode(&onehot(&[0, 1, 1, 0, 1, 2, 2], 4), &cs).unwrap();
        assert_eq!(t, "aab");
        assert!(conf > 0.99);
    }

    #[test]
    fn all_blank_is_empty() {
        let (t, c) = ctc_greedy_decode(&onehot(&[0, 0, 0], 3), &['x', 'y']).unwrap();
        assert_eq!((t.as_str(), c), ("", 0.0));
    }

    #[test]
    fn validates_shapes() {
        assert!(ctc_greedy_decode(&onehot(&[0], 3), &['a']).is_err());
        assert!(ctc_greedy_decode(&Tensor::zeros(vec![2]), &['a']).is_err());
    }
}
