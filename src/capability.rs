//! Capability registry types used by `iron-core`.

use std::collections::HashMap;

/// Stable identifier for a capability family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CapabilityId(pub &'static str);

impl CapabilityId {
    /// Filesystem capability identifier.
    pub const FILESYSTEM: CapabilityId = CapabilityId("iron.filesystem");
    /// Terminal capability identifier.
    pub const TERMINAL: CapabilityId = CapabilityId("iron.terminal");
    /// Shell capability identifier.
    pub const SHELL: CapabilityId = CapabilityId("iron.shell");
}

impl std::fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Describes a capability exposed by the runtime.
#[derive(Debug, Clone)]
pub struct CapabilityDescriptor {
    /// Stable capability identifier.
    pub id: CapabilityId,
    /// Human-readable capability name.
    pub name: &'static str,
    /// Human-readable capability description.
    pub description: &'static str,
    /// Backend selected for this capability.
    pub backend: CapabilityBackend,
    /// Whether using this capability requires permission.
    pub requires_permission: bool,
}

/// Backend implementation used for a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityBackend {
    /// Use the built-in local implementation.
    Local,
    /// Use an ACP client override.
    AcpOverride,
}

/// Decision returned from a capability permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    /// Allow the capability use.
    Allow,
    /// Deny the capability use.
    Deny,
    /// Cancel the enclosing prompt.
    Cancel,
}

/// Permission request emitted for a capability-mediated tool call.
#[derive(Debug, Clone)]
pub struct PermissionRequest {
    /// Unique tool call identifier.
    pub tool_call_id: String,
    /// Tool name requesting the capability.
    pub tool_name: String,
    /// Capability involved in the request, when known.
    pub capability_id: Option<CapabilityId>,
    /// Human-readable request description.
    pub description: String,
    /// Structured tool arguments.
    pub arguments: serde_json::Value,
}

/// Permission response for a capability request.
#[derive(Debug, Clone)]
pub struct PermissionResponse {
    /// Final permission decision.
    pub decision: PermissionDecision,
}

/// Constructors for the built-in capability descriptors.
pub struct BuiltinCapabilities;

impl BuiltinCapabilities {
    /// Filesystem capability descriptor.
    pub fn filesystem() -> CapabilityDescriptor {
        CapabilityDescriptor {
            id: CapabilityId::FILESYSTEM,
            name: "Filesystem",
            description: "Read and write files in the workspace",
            backend: CapabilityBackend::Local,
            requires_permission: true,
        }
    }

    /// Terminal capability descriptor.
    pub fn terminal() -> CapabilityDescriptor {
        CapabilityDescriptor {
            id: CapabilityId::TERMINAL,
            name: "Terminal",
            description: "Execute commands in a terminal",
            backend: CapabilityBackend::Local,
            requires_permission: true,
        }
    }

    /// Shell capability descriptor.
    pub fn shell() -> CapabilityDescriptor {
        CapabilityDescriptor {
            id: CapabilityId::SHELL,
            name: "Shell",
            description: "Execute shell commands",
            backend: CapabilityBackend::Local,
            requires_permission: true,
        }
    }

    /// Return all built-in capability descriptors.
    pub fn all() -> Vec<CapabilityDescriptor> {
        vec![Self::filesystem(), Self::terminal(), Self::shell()]
    }
}

/// Registry of known capabilities.
#[derive(Debug, Default)]
pub struct CapabilityRegistry {
    capabilities: HashMap<CapabilityId, CapabilityDescriptor>,
}

impl CapabilityRegistry {
    /// Create an empty capability registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a capability registry pre-populated with built-ins.
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        for desc in BuiltinCapabilities::all() {
            registry.register(desc);
        }
        registry
    }

    /// Register or replace a capability descriptor.
    pub fn register(&mut self, descriptor: CapabilityDescriptor) {
        self.capabilities.insert(descriptor.id, descriptor);
    }

    /// Look up a capability descriptor.
    pub fn get(&self, id: CapabilityId) -> Option<&CapabilityDescriptor> {
        self.capabilities.get(&id)
    }

    /// Look up a mutable capability descriptor.
    pub fn get_mut(&mut self, id: CapabilityId) -> Option<&mut CapabilityDescriptor> {
        self.capabilities.get_mut(&id)
    }

    /// Iterate over all registered capabilities.
    pub fn iter(&self) -> impl Iterator<Item = (&CapabilityId, &CapabilityDescriptor)> {
        self.capabilities.iter()
    }

    /// Return the number of registered capabilities.
    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    /// Return whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }

    /// Return the backend configured for a capability.
    pub fn backend_for(&self, id: CapabilityId) -> Option<CapabilityBackend> {
        self.capabilities.get(&id).map(|d| d.backend)
    }

    /// Return whether a capability requires permission.
    pub fn requires_permission(&self, id: CapabilityId) -> bool {
        self.capabilities
            .get(&id)
            .map(|d| d.requires_permission)
            .unwrap_or(false)
    }

    /// Return whether a capability is serviced by an ACP override.
    pub fn is_acp_overridden(&self, id: CapabilityId) -> bool {
        self.backend_for(id) == Some(CapabilityBackend::AcpOverride)
    }
}
