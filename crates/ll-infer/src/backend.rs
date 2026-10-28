use crate::{batching, ep::{select_ep, ExecutionProvider}, registry::ModelSpec, tensor::Tensor};
use ll_core::Result;
use std::sync::Arc;

/// A runtime that can execute models. Inputs/outputs carry a leading batch dimension.
pub trait InferenceBackend: Send + Sync {
    fn name(&self) -> &str;
    fn available_eps(&self) -> Vec<ExecutionProvider>;
    fn run(&self, model: &ModelSpec, ep: ExecutionProvider, inputs: &[Tensor]) -> Result<Vec<Tensor>>;
}

type RunFn = dyn Fn(&[Tensor]) -> Result<Vec<Tensor>> + Send + Sync;

/// Closure-backed backend for tests and for wrapping simple pure-Rust "models".
pub struct FnBackend {
    name: String,
    eps: Vec<ExecutionProvider>,
    f: Box<RunFn>,
}

impl FnBackend {
    pub fn new(name: &str, f: impl Fn(&[Tensor]) -> Result<Vec<Tensor>> + Send + Sync + 'static) -> Self {
        FnBackend { name: name.into(), eps: vec![ExecutionProvider::Cpu], f: Box::new(f) }
    }
    pub fn with_eps(mut self, eps: Vec<ExecutionProvider>) -> Self { self.eps = eps; self }
}

impl InferenceBackend for FnBackend {
    fn name(&self) -> &str { &self.name }
    fn available_eps(&self) -> Vec<ExecutionProvider> { self.eps.clone() }
    fn run(&self, _m: &ModelSpec, _ep: ExecutionProvider, inputs: &[Tensor]) -> Result<Vec<Tensor>> { (self.f)(inputs) }
}

/// A model bound to a backend and a chosen execution provider.
pub struct Session {
    backend: Arc<dyn InferenceBackend>,
    spec: ModelSpec,
    ep: ExecutionProvider,
    max_batch: usize,
}

impl Session {
    pub fn new(backend: Arc<dyn InferenceBackend>, spec: ModelSpec, preference: &[ExecutionProvider], max_batch: usize) -> Self {
        let ep = select_ep(preference, &backend.available_eps(), &spec.supported_eps);
        Session { backend, spec, ep, max_batch: max_batch.max(1) }
    }
    pub fn ep(&self) -> ExecutionProvider { self.ep }
    pub fn spec(&self) -> &ModelSpec { &self.spec }

    /// One output tensor per input item, in input order, auto-batched.
    pub fn run_items(&self, items: Vec<Tensor>) -> Result<Vec<Tensor>> {
        batching::run_batched(self.backend.as_ref(), &self.spec, self.ep, items, self.max_batch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Task;

    fn spec() -> ModelSpec {
        ModelSpec { name: "m".into(), version: "1".into(), task: Task::Embed, file: "x".into(), sha256: None, size_bytes: None, license: None, input_shape: vec![], supported_eps: vec![] }
    }

    #[test]
    fn session_selects_ep_and_runs() {
        let b = Arc::new(FnBackend::new("double", |i| {
            Ok(vec![Tensor { shape: i[0].shape.clone(), data: i[0].data.iter().map(|v| v * 2.0).collect() }])
        }).with_eps(vec![ExecutionProvider::CoreMl, ExecutionProvider::Cpu]));
        let s = Session::new(b, spec(), &[ExecutionProvider::Cuda, ExecutionProvider::CoreMl, ExecutionProvider::Cpu], 4);
        assert_eq!(s.ep(), ExecutionProvider::CoreMl);
        let out = s.run_items(vec![Tensor::new(vec![2], vec![1.0, 2.0]).unwrap()]).unwrap();
        assert_eq!(out[0].data, vec![2.0, 4.0]);
    }
}
