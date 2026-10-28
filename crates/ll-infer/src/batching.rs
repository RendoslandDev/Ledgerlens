use crate::{backend::InferenceBackend, ep::ExecutionProvider, registry::ModelSpec, tensor::Tensor};
use ll_core::{Error, Result};
use std::collections::BTreeMap;

/// Stacks equally-shaped tensors `[d..]` into one `[n, d..]` tensor.
pub fn stack(items: &[Tensor]) -> Result<Tensor> {
    let first = items.first().ok_or_else(|| Error::Inference("cannot stack zero tensors".into()))?;
    let mut data = Vec::with_capacity(first.numel() * items.len());
    for t in items {
        if t.shape != first.shape {
            return Err(Error::Inference(format!("shape mismatch when stacking: {:?} vs {:?}", t.shape, first.shape)));
        }
        data.extend_from_slice(&t.data);
    }
    let mut shape = vec![items.len()];
    shape.extend_from_slice(&first.shape);
    Tensor::new(shape, data)
}

/// Splits `[n, d..]` into `n` tensors of shape `[d..]`.
pub fn unstack(t: &Tensor) -> Result<Vec<Tensor>> {
    let n = *t.shape.first().ok_or_else(|| Error::Inference("cannot unstack a scalar".into()))?;
    if n == 0 { return Ok(vec![]); }
    let inner: Vec<usize> = t.shape[1..].to_vec();
    let sz: usize = inner.iter().product();
    (0..n).map(|i| Tensor::new(inner.clone(), t.data[i * sz..(i + 1) * sz].to_vec())).collect()
}

/// Runs `inputs` (one tensor per item) through `backend`, batching same-shaped items together in
/// chunks of at most `max_batch`. Output `i` corresponds to input `i` regardless of grouping.
pub fn run_batched(
    backend: &dyn InferenceBackend,
    model: &ModelSpec,
    ep: ExecutionProvider,
    inputs: Vec<Tensor>,
    max_batch: usize,
) -> Result<Vec<Tensor>> {
    let max_batch = max_batch.max(1);
    let mut groups: BTreeMap<Vec<usize>, Vec<usize>> = BTreeMap::new();
    for (i, t) in inputs.iter().enumerate() { groups.entry(t.shape.clone()).or_default().push(i); }

    let mut out: Vec<Option<Tensor>> = vec![None; inputs.len()];
    for idxs in groups.values() {
        for chunk in idxs.chunks(max_batch) {
            let batch: Vec<Tensor> = chunk.iter().map(|&i| inputs[i].clone()).collect();
            let stacked = stack(&batch)?;
            let res = backend.run(model, ep, &[stacked])?;
            let first = res.into_iter().next().ok_or_else(|| Error::Inference("backend returned no outputs".into()))?;
            let parts = unstack(&first)?;
            if parts.len() != chunk.len() {
                return Err(Error::Inference(format!("backend returned {} rows for a batch of {}", parts.len(), chunk.len())));
            }
            for (&i, p) in chunk.iter().zip(parts) { out[i] = Some(p); }
        }
    }
    Ok(out.into_iter().map(|o| o.expect("every index filled")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{backend::FnBackend, registry::Task};
    use std::sync::{atomic::{AtomicUsize, Ordering}, Arc};

    fn spec() -> ModelSpec {
        ModelSpec { name: "m".into(), version: "1".into(), task: Task::Embed, file: "x".into(), sha256: None, size_bytes: None, license: None, input_shape: vec![], supported_eps: vec![] }
    }
    fn t(shape: Vec<usize>, v: f32) -> Tensor { let n = shape.iter().product(); Tensor::new(shape, vec![v; n]).unwrap() }

    #[test]
    fn stack_roundtrip() {
        let items = vec![t(vec![2], 1.0), t(vec![2], 2.0)];
        let s = stack(&items).unwrap();
        assert_eq!(s.shape, vec![2, 2]);
        assert_eq!(unstack(&s).unwrap(), items);
        assert!(stack(&[t(vec![2], 1.0), t(vec![3], 1.0)]).is_err());
    }

    #[test]
    fn preserves_order_across_shape_groups_and_chunks() {
        let calls = Arc::new(AtomicUsize::new(0));
        let c2 = calls.clone();
        // "model": adds 100 to each value; keeps batch dim
        let be = FnBackend::new("add", move |i| {
            c2.fetch_add(1, Ordering::SeqCst);
            Ok(vec![Tensor { shape: i[0].shape.clone(), data: i[0].data.iter().map(|v| v + 100.0).collect() }])
        });
        let inputs = vec![t(vec![2], 1.0), t(vec![3], 2.0), t(vec![2], 3.0), t(vec![2], 4.0), t(vec![3], 5.0)];
        let out = run_batched(&be, &spec(), ExecutionProvider::Cpu, inputs, 2).unwrap();
        let firsts: Vec<f32> = out.iter().map(|x| x.data[0]).collect();
        assert_eq!(firsts, vec![101.0, 102.0, 103.0, 104.0, 105.0]);
        assert_eq!(out[1].shape, vec![3]);
        // shape[2]: 3 items -> 2 chunks, shape[3]: 2 items -> 1 chunk
        assert_eq!(calls.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn detects_bad_backend_output() {
        let be = FnBackend::new("bad", |_| Ok(vec![Tensor::zeros(vec![1, 2])]));
        let r = run_batched(&be, &spec(), ExecutionProvider::Cpu, vec![t(vec![2], 0.0), t(vec![2], 0.0)], 8);
        assert!(r.is_err());
    }
}
