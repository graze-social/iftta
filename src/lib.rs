//! # ifthisthenat
//!
//! ifthisthenat is an AT Protocol automation service built in Rust that processes events from
//! Jetstream (Bluesky's firehose), webhooks, and periodic schedules. It uses a blueprint-based
//! system with DataLogic expressions for rule evaluation and supports various actions like
//! publishing records and sending webhooks.
//!
//! ## Architecture Overview
//!
//! The service is built around several core components:
//!
//! ### Blueprint System
//! - **Blueprints** define automation workflows with ordered node evaluation
//! - Each blueprint has an AT-URI identifier and evaluation order
//! - Blueprints are cached in memory/Redis for performance
//!
//! ### Node Types
//! - **Entry nodes**: `jetstream_entry`, `webhook_entry`, `periodic_entry` - filter incoming events
//! - **Processing nodes**: `condition`, `transform`, `facet_text` - modify or filter data
//! - **Action nodes**: `publish_record`, `publish_webhook`, `debug_action` - perform side effects
//!
//! ### Evaluation Engine
//! - Uses `datalogic-rs` for expression evaluation
//! - Sequential node processing with early termination on failure
//! - Async evaluation via queue adapters (memory/Redis)
//!
//! ### Consumer System
//! - Handles Jetstream events with partitioning support
//! - Multi-worker thread processing
//! - Redis or file-based cursor persistence
//!
//! ## Configuration
//!
//! The service is configured via environment variables. Key variables include:
//! - `EXTERNAL_BASE`: Service base URL
//! - `HTTP_COOKIE_KEY`: Cookie encryption key
//! - `ISSUER_DID`: Service DID
//! - `JETSTREAM_ENABLED`: Enable Jetstream consumer
//! - `REDIS_URL`: Redis connection (optional, enables advanced features)
//!
//! ## Error Handling
//!
//! All error strings use the format: `error-iftta-<domain>-<number> <message>: <details>`
//!
//! ## Examples
//!
//! ```rust,ignore
//! use ifthisthenat::{config::Config, processor::BlueprintProcessor};
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Load configuration from environment
//!     let config = Config::new()?;
//!
//!     // Set up storage and queue adapters
//!     // ... setup code ...
//!
//!     // Create and start the blueprint processor
//!     let processor = BlueprintProcessor::new(
//!         blueprint_storage,
//!         node_storage,
//!         blueprint_sender,
//!         throttler,
//!     );
//!
//!     processor.start_processing(event_receiver).await?;
//!
//!     Ok(())
//! }
//! ```

pub(crate) mod atproto;

/// Configuration management for the ifthisthenat service.
///
/// This module contains configuration structures and loading logic for all
/// service components including HTTP, database, Jetstream, Redis, and more.
/// Configuration is primarily loaded from environment variables.
pub mod config;

pub(crate) mod constants;

/// Event consumer system for processing Jetstream and other event sources.
///
/// Handles event consumption from AT Protocol Jetstream with support for
/// partitioning, cursor persistence, and multi-worker processing.
pub mod consumer;

/// Denylist functionality for filtering unwanted content or users.
///
/// Provides configurable denylist implementations including no-op and
/// PostgreSQL-backed denylists for content moderation.
pub mod denylist;

/// Service access token manager for AT Protocol authentication.
///
/// Re-exported from the atproto module for convenience in binary usage.
pub use atproto::auth::ServiceAccessTokenManager;

/// Blueprint evaluation engine with node types and evaluators.
///
/// Contains the core evaluation logic for processing data through blueprint
/// nodes, including entry nodes, condition nodes, transform nodes, and action nodes.
pub mod engine;

pub(crate) mod errors;

/// HTTP server and API endpoints for the ifthisthenat service.
///
/// Provides XRPC endpoints for blueprint management and OAuth authentication
/// for user access control.
pub mod http;

/// Identity resolution and caching for AT Protocol DIDs.
///
/// Handles DID resolution with configurable caching to improve performance
/// when resolving user identities frequently.
pub mod identity_cache;

/// Distributed leadership election for coordinating multiple service instances.
///
/// Enables coordination between multiple service instances for tasks like
/// blueprint evaluation, webhook processing, and scheduling.
pub mod leadership;

/// Metrics collection and monitoring for service observability.
///
/// Provides instrumentation for tracking service performance, throughput,
/// and health metrics.
pub mod metrics;

/// Blueprint processing and event routing logic.
///
/// The core processor that filters incoming events through cached blueprints
/// and routes matching events to the async evaluation system.
pub mod processor;

/// Prototype service functionality (experimental features).
///
/// Contains experimental or prototype features that may be promoted to
/// stable modules in the future.
pub mod prototype_service;

/// Queue adapter abstractions for different queue backends.
///
/// Provides abstractions for blueprint work queues with implementations
/// for in-memory MPSC channels and Redis-based queues.
pub mod queue_adapter;

/// Serialization utilities for data interchange.
///
/// Common serialization and deserialization utilities used throughout
/// the service for consistent data handling.
pub mod serialization;

/// Storage layer abstractions and implementations.
///
/// Provides storage traits and implementations for blueprints, nodes,
/// sessions, and other persistent data with PostgreSQL backing.
pub mod storage;

/// Background task management and execution.
///
/// Handles async task execution for blueprint evaluation, webhook delivery,
/// scheduling, and other background operations.
pub mod tasks;

/// Rate limiting and throttling for blueprint evaluations.
///
/// Provides configurable throttling mechanisms to prevent abuse and
/// ensure fair resource usage across users and blueprints.
pub mod throttle;

/// Data validation utilities for user input and configuration.
///
/// Common validation logic for ensuring data integrity and security
/// throughout the service.
pub mod validation;

#[cfg(test)]
pub mod test_helpers;
