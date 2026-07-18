#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

pub mod discovery;
pub mod manifest;
pub mod registry;
pub mod types;

#[cfg(test)]
mod tests;

use discovery::discover_all;
use registry::PluginRegistry;

#[derive(Debug)]
struct RegistryHost {
    workspace: PathBuf,
    registry: PluginRegistry,
}

static REGISTRY: OnceLock<Mutex<RegistryHost>> = OnceLock::new();

/// Build the one process-wide plugin snapshot before any launch surface can
/// construct Skills or MCP. Repeated initialization replaces the snapshot so
/// tests and explicit reloads cannot strand a stale workspace registry.
pub fn init_registry(workspace: &Path) {
    let host = RegistryHost {
        workspace: workspace.to_path_buf(),
        registry: discover_all(workspace),
    };
    if let Some(lock) = REGISTRY.get() {
        if let Ok(mut current) = lock.lock() {
            *current = host;
        }
    } else {
        let _ = REGISTRY.set(Mutex::new(host));
    }
}

pub fn reload_registry(workspace: &Path) -> Result<usize, String> {
    let registry = discover_all(workspace);
    let count = registry.len();
    let host = RegistryHost {
        workspace: workspace.to_path_buf(),
        registry,
    };
    if let Some(lock) = REGISTRY.get() {
        let mut current = lock
            .lock()
            .map_err(|_| "Plugin registry lock is poisoned".to_string())?;
        *current = host;
    } else {
        REGISTRY
            .set(Mutex::new(host))
            .map_err(|_| "Plugin registry initialization raced".to_string())?;
    }
    Ok(count)
}

#[must_use]
pub fn registry_workspace() -> Option<PathBuf> {
    REGISTRY
        .get()
        .and_then(|lock| lock.lock().ok().map(|host| host.workspace.clone()))
}

pub fn try_with_registry<R>(f: impl FnOnce(&PluginRegistry) -> R) -> Option<R> {
    REGISTRY
        .get()
        .and_then(|lock| lock.lock().ok().map(|host| f(&host.registry)))
}

pub fn with_registry<R>(f: impl FnOnce(&mut PluginRegistry) -> R) -> Option<R> {
    REGISTRY
        .get()
        .and_then(|lock| lock.lock().ok().map(|mut host| f(&mut host.registry)))
}
