//! Inference plumbing that is independent of any one runtime.
//!
//! An ONNX Runtime (`ort`) backend plugs in by implementing [`InferenceBackend`]; everything else
//! here (registry, EP selection, batching) is runtime-agnostic and unit-tested without native libs.

pub mod backend;
pub mod batching;
pub mod ep;
pub mod registry;
pub mod tensor;

pub use backend::{FnBackend, InferenceBackend, Session};
pub use batching::{run_batched, stack, unstack};
pub use ep::{default_preference, select_ep, ExecutionProvider};
pub use registry::{ModelManifest, ModelRegistry, ModelSpec, Task};
pub use tensor::Tensor;
