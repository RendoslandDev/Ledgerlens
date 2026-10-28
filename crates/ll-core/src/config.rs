//! Config shapes and defaults only. Loading from disk/env happens in the app (that is I/O).
use crate::{currency::Currency, error::{Error, Result}, fields::FieldName};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Paths { pub data_dir: PathBuf, pub models_dir: PathBuf, pub watch_dirs: Vec<PathBuf> }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InferenceConfig {
    pub intra_threads: usize,
    pub max_concurrent_heavy: usize,
    pub vlm_idle_unload_secs: u64,
    pub prefer_gpu: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PipelineConfig {
    /// Escalate to the VLM when min critical-field confidence falls below this.
    pub escalate_below: f32,
    pub default_currency: String,
    pub field_thresholds: HashMap<FieldName, f32>,
    pub render_dpi: u32,
    pub max_page_pixels: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrivacyConfig { pub allow_network: bool, pub sync_enabled: bool }

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub paths: Paths,
    pub inference: InferenceConfig,
    pub pipeline: PipelineConfig,
    pub privacy: PrivacyConfig,
}

impl Config {
    pub fn with_data_dir(data_dir: PathBuf) -> Self {
        Self {
            paths: Paths { models_dir: data_dir.join("models"), data_dir, watch_dirs: vec![] },
            inference: InferenceConfig { intra_threads: 4, max_concurrent_heavy: 1, vlm_idle_unload_secs: 300, prefer_gpu: true },
            pipeline: PipelineConfig { escalate_below: 0.85, default_currency: "USD".into(), field_thresholds: HashMap::new(), render_dpi: 250, max_page_pixels: 16_000_000 },
            privacy: PrivacyConfig { allow_network: false, sync_enabled: false },
        }
    }

    /// Fail at startup with a clear message instead of misbehaving mid-pipeline.
    pub fn validate(&self) -> Result<()> {
        let bad = |m: &str| Err(Error::Invalid(m.to_string()));
        if self.paths.data_dir.as_os_str().is_empty() { return bad("data_dir must not be empty"); }
        if self.inference.intra_threads == 0 { return bad("intra_threads must be >= 1"); }
        if self.inference.max_concurrent_heavy == 0 { return bad("max_concurrent_heavy must be >= 1"); }
        if !(0.0..=1.0).contains(&self.pipeline.escalate_below) { return bad("escalate_below must be in 0..=1"); }
        if self.pipeline.field_thresholds.values().any(|t| !(0.0..=1.0).contains(t)) { return bad("field threshold out of range"); }
        if !(72..=600).contains(&self.pipeline.render_dpi) { return bad("render_dpi must be in 72..=600"); }
        if Currency::new(&self.pipeline.default_currency).is_err() { return bad("default_currency must be a supported ISO code"); }
        if self.privacy.sync_enabled && !self.privacy.allow_network { return bad("sync requires allow_network"); }
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self { Self::with_data_dir(PathBuf::from("ledgerlens-data")) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_validate() { Config::with_data_dir("/tmp/ll".into()).validate().unwrap(); }
    #[test]
    fn rejects_bad_values() {
        let mut c = Config::with_data_dir("/tmp/ll".into());
        c.pipeline.escalate_below = 1.5;
        assert!(c.validate().is_err());
        let mut c = Config::with_data_dir("/tmp/ll".into());
        c.privacy.sync_enabled = true;
        assert!(c.validate().is_err(), "sync without network permission must be rejected");
    }
}
