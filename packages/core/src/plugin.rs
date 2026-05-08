//! Plugin system — manifest, lifecycle, registry, and execution context.
//!
//! # Plugin Lifecycle
//!
//! ```text
//! Discovered ──► Loaded ──► on_load() ──► Running
//!                                              │
//!                                        on_unload()
//!                                              │
//!                                           Stopped
//! ```
//!
//! Plugins are discovered from the `plugins/` directory on startup. Each
//! plugin directory must contain a `plugin.toml` manifest file.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{error::Result, event::EventBus, storage::StorageProvider};

// ---------------------------------------------------------------------------
// Plugin Manifest
// ---------------------------------------------------------------------------

/// The `plugin.toml` manifest — every plugin must provide one.
///
/// # Example `plugin.toml`
/// ```toml
/// [plugin]
/// id      = "storage-s3"
/// name    = "Amazon S3 Storage"
/// version = "1.0.0"
/// api_version = "^1.0"
/// author  = "FreeBox Team"
/// license = "Apache-2.0"
///
/// [plugin.capabilities]
/// provides = ["storage.provider"]
/// requires = []
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    /// Unique, stable identifier (e.g. `"storage-s3"`). Used in config files.
    pub id: String,

    /// Human-readable display name.
    pub name: String,

    /// Plugin version in SemVer format.
    pub version: String,

    /// Minimum kernel API version required (SemVer range, e.g. `"^1.0"`).
    pub api_version: String,

    /// Optional author information.
    pub author: Option<String>,

    /// SPDX license identifier.
    pub license: Option<String>,

    /// Declared capabilities — what this plugin provides and requires.
    pub capabilities: Capabilities,

    /// JSON Schema for plugin-specific configuration. Validated at load time.
    /// Stored as raw JSON so the core crate has no schema-validation dependency.
    #[serde(default)]
    pub config_schema: serde_json::Value,
}

/// Declared capability contract for a plugin.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Capabilities {
    /// Capability IDs this plugin provides (e.g. `["storage.provider"]`).
    #[serde(default)]
    pub provides: Vec<String>,

    /// Capability IDs this plugin requires from the runtime.
    #[serde(default)]
    pub requires: Vec<String>,
}

// ---------------------------------------------------------------------------
// Plugin Context
// ---------------------------------------------------------------------------

/// The execution context injected into every plugin at load time.
///
/// Plugins use this to access kernel services (event bus, config, logging)
/// without depending on the kernel crate itself. This is the **dependency
/// inversion** that makes plugins independently testable.
pub struct PluginContext {
    /// Typed event bus for publishing and subscribing to platform events.
    pub event_bus: std::sync::Arc<dyn EventBus>,

    /// Plugin-specific configuration (already validated against schema).
    pub config: HashMap<String, serde_json::Value>,

    /// Unique runtime ID for this plugin instance (regenerated each boot).
    pub instance_id: Uuid,
}

impl PluginContext {
    /// Retrieve a required config value, returning an error if missing.
    pub fn require_config(&self, key: &str) -> Result<&serde_json::Value> {
        self.config
            .get(key)
            .ok_or_else(|| crate::error::Error::config(format!("required key `{key}` not found")))
    }

    /// Retrieve an optional config string value.
    pub fn config_str(&self, key: &str) -> Option<&str> {
        self.config.get(key)?.as_str()
    }
}

// ---------------------------------------------------------------------------
// Plugin Trait
// ---------------------------------------------------------------------------

/// The core trait every FreeBox plugin must implement.
///
/// The kernel discovers plugins, calls [`on_load`] once at startup, and
/// [`on_unload`] during graceful shutdown. Everything in between is driven
/// by the event bus.
#[async_trait::async_trait]
pub trait Plugin: Send + Sync {
    /// Returns this plugin's static manifest (parsed from `plugin.toml`).
    fn manifest(&self) -> &PluginManifest;

    /// Called once when the plugin is loaded. Use this to:
    /// - Validate configuration
    /// - Subscribe to events
    /// - Register provided capabilities with the kernel
    ///
    /// If this returns an error the plugin is disabled and the error is logged.
    async fn on_load(&self, ctx: &PluginContext) -> Result<()>;

    /// Called during graceful shutdown. Implementations must:
    /// - Flush any pending writes / queues
    /// - Release external resources (file handles, connections)
    ///
    /// The Runtime gives each plugin a 30-second budget before force-killing.
    async fn on_unload(&self) -> Result<()>;

    /// Returns `true` if the plugin provides a [`StorageProvider`].
    /// Kernel uses this to decide whether to request `as_storage_provider()`.
    fn is_storage_provider(&self) -> bool {
        false
    }

    /// Downcast to a [`StorageProvider`] reference.
    ///
    /// Override this if `is_storage_provider()` returns `true`.
    fn as_storage_provider(&self) -> Option<&dyn StorageProvider> {
        None
    }
}

// ---------------------------------------------------------------------------
// Plugin Registry
// ---------------------------------------------------------------------------

/// Runtime registry of all loaded plugins.
///
/// The registry is append-only after startup to avoid synchronisation
/// complexity. Plugins are never removed while the server is running
/// (a reload requires a restart — this is intentional for security auditability).
pub struct PluginRegistry {
    plugins: Vec<Box<dyn Plugin>>,
}

impl PluginRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    /// Register a plugin. Called during the bootstrap phase before the server
    /// starts accepting connections.
    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        tracing::info!(
            id = %plugin.manifest().id,
            version = %plugin.manifest().version,
            "Plugin registered"
        );
        self.plugins.push(plugin);
    }

    /// Find the first registered `StorageProvider` plugin.
    ///
    /// In a multi-provider future this will accept a provider ID. For now
    /// the platform uses whatever storage backend was registered first.
    pub fn storage_provider(&self) -> Option<&dyn StorageProvider> {
        self.plugins
            .iter()
            .find(|p| p.is_storage_provider())
            .and_then(|p| p.as_storage_provider())
    }

    /// Returns references to all registered plugins (read-only).
    pub fn all(&self) -> &[Box<dyn Plugin>] {
        &self.plugins
    }

    /// Call `on_load` on every registered plugin in registration order.
    ///
    /// Plugins that fail to load are logged and skipped — remaining plugins
    /// continue to load. Returns `Ok(())` even if some plugins failed.
    pub async fn load_all(&self, ctx: &PluginContext) -> Result<()> {
        for plugin in &self.plugins {
            if let Err(e) = plugin.on_load(ctx).await {
                tracing::error!(plugin = %plugin.manifest().id, error = %e, "Plugin load failed — skipping");
            }
        }
        Ok(())
    }

    /// Call `on_unload` on every plugin in **reverse** registration order
    /// (LIFO — dependencies unload after dependents).
    pub async fn unload_all(&self) -> Result<()> {
        for plugin in self.plugins.iter().rev() {
            if let Err(e) = plugin.on_unload().await {
                tracing::error!(plugin = %plugin.manifest().id, error = %e, "Plugin unload error");
            }
        }
        Ok(())
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::NoopEventBus;
    use std::sync::Arc;

    /// A minimal test plugin for unit testing the registry.
    struct TestPlugin {
        manifest: PluginManifest,
        fail_on_load: bool,
    }

    impl TestPlugin {
        fn new(id: &str) -> Self {
            Self {
                manifest: PluginManifest {
                    id: id.to_string(),
                    name: id.to_string(),
                    version: "1.0.0".to_string(),
                    api_version: "^1.0".to_string(),
                    author: None,
                    license: None,
                    capabilities: Capabilities::default(),
                    config_schema: serde_json::Value::Null,
                },
                fail_on_load: false,
            }
        }

        fn failing(id: &str) -> Self {
            Self {
                fail_on_load: true,
                ..Self::new(id)
            }
        }
    }

    #[async_trait::async_trait]
    impl Plugin for TestPlugin {
        fn manifest(&self) -> &PluginManifest {
            &self.manifest
        }

        async fn on_load(&self, _ctx: &PluginContext) -> Result<()> {
            if self.fail_on_load {
                Err(crate::error::Error::plugin_load(
                    &self.manifest.id,
                    "intentional test failure",
                ))
            } else {
                Ok(())
            }
        }

        async fn on_unload(&self) -> Result<()> {
            Ok(())
        }
    }

    fn test_context() -> PluginContext {
        PluginContext {
            event_bus: Arc::new(NoopEventBus),
            config: std::collections::HashMap::new(),
            instance_id: Uuid::new_v4(),
        }
    }

    #[test]
    fn registry_starts_empty() {
        let registry = PluginRegistry::new();
        assert!(registry.all().is_empty());
        assert!(registry.storage_provider().is_none());
    }

    #[test]
    fn register_and_retrieve_plugin() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin::new("test-plugin")));
        assert_eq!(registry.all().len(), 1);
        assert_eq!(registry.all()[0].manifest().id, "test-plugin");
    }

    #[tokio::test]
    async fn load_all_succeeds_with_good_plugins() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin::new("a")));
        registry.register(Box::new(TestPlugin::new("b")));
        let ctx = test_context();
        let result = registry.load_all(&ctx).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn load_all_continues_after_failure() {
        let mut registry = PluginRegistry::new();
        registry.register(Box::new(TestPlugin::new("a")));
        registry.register(Box::new(TestPlugin::failing("bad")));
        registry.register(Box::new(TestPlugin::new("c")));
        let ctx = test_context();
        // Should succeed even though "bad" failed — it gets logged and skipped.
        let result = registry.load_all(&ctx).await;
        assert!(result.is_ok());
    }

    #[test]
    fn plugin_context_require_config_missing() {
        let ctx = test_context();
        assert!(ctx.require_config("nonexistent").is_err());
    }

    #[test]
    fn plugin_context_require_config_present() {
        let mut ctx = test_context();
        ctx.config.insert("key".into(), serde_json::json!("value"));
        let val = ctx.require_config("key").unwrap();
        assert_eq!(val.as_str().unwrap(), "value");
    }
}
