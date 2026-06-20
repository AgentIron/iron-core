//! Store-backed provider profile management.
//!
//! This module provides durable storage, validation, import/export, and
//! effective registry construction for provider profiles. Built-in provider
//! profiles remain compiled into `iron-providers`; persisted rows in the config
//! store represent only custom profiles and explicit overrides.

pub mod registry;
pub mod store;
pub mod validation;

pub use registry::build_effective_registry;
pub use store::{
    export_provider_profile, import_provider_profile, DurableProfileStore, ProfileStore,
};
pub use validation::validate_provider_profile;
