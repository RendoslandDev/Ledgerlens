use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionProvider { TensorRt, Cuda, CoreMl, DirectMl, OpenVino, Cpu }

impl ExecutionProvider {
    pub fn all() -> Vec<ExecutionProvider> {
        use ExecutionProvider::*;
        vec![TensorRt, Cuda, CoreMl, DirectMl, OpenVino, Cpu]
    }
}

/// Preferred provider order for an OS (`std::env::consts::OS` values). CPU is always last.
pub fn default_preference(os: &str) -> Vec<ExecutionProvider> {
    use ExecutionProvider::*;
    match os {
        "macos" => vec![CoreMl, Cpu],
        "windows" => vec![TensorRt, Cuda, DirectMl, OpenVino, Cpu],
        _ => vec![TensorRt, Cuda, OpenVino, Cpu],
    }
}

/// Picks the first provider in `preference` that the runtime `available`ly offers and the model
/// `supported` list allows (an empty `supported` list means "any"). Falls back to CPU.
pub fn select_ep(
    preference: &[ExecutionProvider],
    available: &[ExecutionProvider],
    supported: &[ExecutionProvider],
) -> ExecutionProvider {
    preference
        .iter()
        .copied()
        .find(|p| available.contains(p) && (supported.is_empty() || supported.contains(p)))
        .unwrap_or(ExecutionProvider::Cpu)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ExecutionProvider::*;

    #[test]
    fn picks_first_available_and_supported() {
        let pref = default_preference("windows");
        assert_eq!(select_ep(&pref, &[DirectMl, Cpu], &[]), DirectMl);
        assert_eq!(select_ep(&pref, &[Cuda, DirectMl, Cpu], &[DirectMl, Cpu]), DirectMl);
    }
    #[test]
    fn falls_back_to_cpu() {
        assert_eq!(select_ep(&default_preference("linux"), &[], &[]), Cpu);
        assert_eq!(select_ep(&[CoreMl], &[CoreMl], &[Cuda]), Cpu);
    }
    #[test]
    fn mac_prefers_coreml() { assert_eq!(default_preference("macos")[0], CoreMl); }
}
