use std::collections::HashMap;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct ActorSizeMetrics {
    pub actor_name: String,
    pub wasm_size: usize,
    pub compressed_size: usize,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BundleMetrics {
    pub total_size: usize,
    pub compressed_size: usize,
    pub actor_metrics: HashMap<String, ActorSizeMetrics>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

impl BundleMetrics {
    pub fn new() -> Self {
        BundleMetrics {
            total_size: 0,
            compressed_size: 0,
            actor_metrics: HashMap::new(),
            timestamp: chrono::Utc::now(),
        }
    }

    pub fn record_actor_size(&mut self, name: &str, wasm_size: usize, compressed_size: usize) {
        self.actor_metrics.insert(name.to_string(), ActorSizeMetrics {
            actor_name: name.to_string(),
            wasm_size,
            compressed_size,
            timestamp: chrono::Utc::now(),
        });
    }

    pub fn save_metrics(&self, path: &std::path::Path) -> std::io::Result<()> {
        let metrics_json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, metrics_json)
    }
} 