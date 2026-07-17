//! Store-backed provider profile management.
//!
//! This module provides durable storage, validation, import/export, and
//! effective registry construction for provider profiles. Built-in provider
//! profiles remain compiled into `iron-providers`; persisted rows in the config
//! store represent only custom profiles and explicit overrides.
//!
//! ```
//! use iron_core::provider_profile::validate_provider_profile;
//! use iron_providers::{ApiFamily, ProviderProfile};
//!
//! let profile = ProviderProfile::new(
//!     "internal-openai",
//!     ApiFamily::Responses,
//!     "https://models.example.com/v1",
//! );
//! let json = serde_json::to_string(&profile)?;
//! let validated = validate_provider_profile(&json)?;
//! assert_eq!(validated.slug, "internal-openai");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod registry;
pub mod store;
pub mod validation;

pub use registry::build_effective_registry;
pub use store::{
    export_provider_profile, import_provider_profile, DurableProfileStore, ProfileStore,
};
pub use validation::validate_provider_profile;
