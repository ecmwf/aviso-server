//! ECPDS destination authorization plugin for `aviso-server`.
//!
//! This crate decides whether a given user is allowed to read a given
//! ECPDS destination, by consulting one or more ECPDS monitor servers
//! and caching the result. It is consumed by `aviso-server` behind the
//! `ecpds` Cargo feature; deployments that don't need ECPDS auth
//! compile without this crate at all.
//!
//! See `aviso-ecpds/README.md` for an architectural overview, the
//! "ECPDS Destination Authorization" section in the operator
//! documentation for setup, and the "ECPDS Plugin Runbook" for on-call
//! triage.
//!
//! Public surface, at a glance:
//!
//! - [`config::EcpdsConfig`] — serde-deserialised configuration.
//! - [`config::PartialOutagePolicy`] — strict (default) vs any-success
//!   merge across multiple ECPDS servers.
//! - [`checker::EcpdsChecker`] — the single facade. `new` is fallible;
//!   `check_access` returns [`checker::AccessCheckResult`] (cache
//!   outcome plus authorisation result) so the route layer can label
//!   hit/miss and fetch metrics on every code path.
//! - [`client::EcpdsError`] / [`client::FetchOutcome`] /
//!   [`client::DenyReason`] — domain error type and typed sub-reasons
//!   with stable Prometheus label strings.

#![warn(missing_docs)]

/// In-process single-flight bounded cache of authorised ECPDS
/// destination lists, keyed by username.
pub mod cache;

/// The single public facade combining the HTTP client, the cache, and
/// the destination match logic.
pub mod checker;

/// HTTP client to one or more ECPDS servers, plus the typed error /
/// fetch-outcome / deny-reason types consumed by the route layer.
pub mod client;

/// Static configuration for the ECPDS plugin (deserialised from YAML
/// at startup).
pub mod config;

pub use checker::EcpdsChecker;
pub use client::EcpdsError;
