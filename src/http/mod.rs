//! HTTP server and API endpoints for the ifthisthenat service.
//!
//! This module provides the complete HTTP interface for the ifthisthenat service,
//! including web UI, REST APIs, XRPC endpoints, and OAuth authentication.
//!
//! # Architecture
//!
//! The HTTP layer is built using Axum and provides:
//! - Web-based dashboard for blueprint management
//! - XRPC endpoints for AT Protocol compatibility
//! - OAuth flow for user authentication
//! - REST API for webhook and prototype management
//! - Static file serving for web assets
//!
//! # API Endpoints
//!
//! ## XRPC Endpoints (AT Protocol)
//! - `GET /xrpc/tools.graze.ifthisthenat.getBlueprints` - List user blueprints
//! - `GET /xrpc/tools.graze.ifthisthenat.getBlueprint` - Get specific blueprint
//! - `POST /xrpc/tools.graze.ifthisthenat.updateBlueprint` - Create/update blueprint
//! - `POST /xrpc/tools.graze.ifthisthenat.deleteBlueprint` - Delete blueprint
//!
//! ## Web UI
//! - `/` - Main dashboard
//! - `/auth/*` - Authentication flows
//! - `/blueprints/*` - Blueprint management
//! - `/webhooks/*` - Webhook management
//!
//! ## Authentication
//!
//! The service uses OAuth with AT Protocol providers for user authentication.
//! Session management is handled via secure cookies.
//!
//! # Error Handling
//!
//! All API endpoints use consistent error handling with appropriate HTTP status
//! codes and JSON error responses. Web UI endpoints redirect with user-friendly
//! error messages.
//!
//! # Examples
//!
//! ```rust,ignore
//! use ifthisthenat::http::{create_app, HttpAppState};
//! use ifthisthenat::config::Config;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let config = Config::new()?;
//!
//!     // Create application state
//!     let state = HttpAppState::new(
//!         config.clone(),
//!         blueprint_storage,
//!         node_storage,
//!         session_storage,
//!         // ... other dependencies
//!     );
//!
//!     // Create and run server
//!     let app = create_app(state);
//!     let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
//!     axum::serve(listener, app).await?;
//!
//!     Ok(())
//! }
//! ```

pub(crate) mod aip_utils;
pub(crate) mod auth;
pub(crate) mod blueprint_helpers;

/// HTTP context and application state management.
pub mod context;

pub(crate) mod errors;
pub(crate) mod handle_auth;
pub(crate) mod handle_blueprints;
pub(crate) mod handle_dashboard;
pub(crate) mod handle_documentation;
pub(crate) mod handle_index;
pub(crate) mod handle_prototypes;
pub(crate) mod handle_public_prototypes;
pub(crate) mod handle_webhooks;
pub(crate) mod handle_wellknown;
pub(crate) mod handle_xrpc;
pub(crate) mod handler_app_password;
pub(crate) mod middleware_auth;

/// HTTP server configuration and setup.
pub mod server;

/// Template rendering and web UI components.
pub mod templates;

pub use context::*;
pub use server::*;
