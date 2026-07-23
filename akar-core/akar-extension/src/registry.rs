//! Extension registry — manages registered extensions and their lifecycle.
//!
//! Extensions can be registered at compile time and are loaded during
//! database initialization.

use crate::Extension;
use crate::context::ExtensionContext;
use std::collections::HashMap;

/// The extension registry manages all loaded extensions.
#[derive(Default)]
pub struct ExtensionRegistry {
    extensions: Vec<Box<dyn Extension>>,
    loaded: HashMap<String, bool>,
}

impl ExtensionRegistry {
    /// Create a new empty extension registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an extension for loading.
    pub fn register(&mut self, extension: Box<dyn Extension>) {
        let name = extension.name().to_string();
        tracing::info!("Registered extension: {}", name);
        self.extensions.push(extension);
        self.loaded.insert(name, false);
    }

    /// Load all registered extensions.
    pub fn load_all(&mut self, context: &ExtensionContext) -> Vec<(String, Result<(), String>)> {
        let mut results = Vec::new();
        for ext in &self.extensions {
            let name = ext.name();
            match ext.load(context) {
                Ok(()) => {
                    tracing::info!("Loaded extension: {}", name);
                    self.loaded.insert(name.to_string(), true);
                    results.push((name.to_string(), Ok(())));
                }
                Err(e) => {
                    tracing::error!("Failed to load extension {}: {}", name, e);
                    results.push((name.to_string(), Err(e)));
                }
            }
        }
        results
    }

    /// Check if a specific extension is loaded.
    pub fn is_loaded(&self, name: &str) -> bool {
        self.loaded.get(name).copied().unwrap_or(false)
    }

    /// Get the number of registered extensions.
    pub fn num_registered(&self) -> usize {
        self.extensions.len()
    }

    /// Get the names of all registered extensions.
    pub fn names(&self) -> Vec<String> {
        self.extensions.iter().map(|e| e.name().to_string()).collect()
    }

    /// Get the number of successfully loaded extensions.
    pub fn num_loaded(&self) -> usize {
        self.loaded.values().filter(|&&v| v).count()
    }

    /// Get the names of all registered extensions.
    pub fn extension_names(&self) -> Vec<&str> {
        self.extensions.iter().map(|e| e.name()).collect()
    }
}
