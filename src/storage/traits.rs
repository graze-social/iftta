//! Storage layer trait definitions and common types.
//!
//! This module defines the core storage abstractions used throughout the ifthisthenat
//! service. It provides a common interface for health checking and error handling
//! across different storage implementations.

use crate::errors::StorageError;
use async_trait::async_trait;

/// Result type alias for storage operations.
///
/// This is a convenience type alias that wraps [`Result`] with [`StorageError`]
/// as the error type. All storage operations should return this type for
/// consistent error handling.
///
/// # Examples
///
/// ```rust,ignore
/// use ifthisthenat::storage::traits::StorageResult;
/// use ifthisthenat::storage::blueprint::Blueprint;
///
/// async fn get_blueprint(id: &str) -> StorageResult<Option<Blueprint>> {
///     // Implementation that may return StorageError
///     Ok(None)
/// }
/// ```
pub type StorageResult<T> = Result<T, StorageError>;

/// Core storage trait for health monitoring and basic operations.
///
/// This trait defines the fundamental operations that all storage implementations
/// must support. It focuses on health checking and monitoring capabilities that
/// are essential for service reliability.
///
/// All storage implementations must be `Send + Sync` to work properly in the
/// async runtime environment with multiple concurrent operations.
///
/// # Thread Safety
///
/// Implementors must ensure that all operations are thread-safe and can be
/// called concurrently from multiple async tasks.
///
/// # Error Handling
///
/// All methods return [`StorageResult<T>`] which wraps potential [`StorageError`]s.
/// Implementations should use appropriate error types from the errors module
/// to provide meaningful error information.
///
/// # Examples
///
/// ```rust,ignore
/// use async_trait::async_trait;
/// use ifthisthenat::storage::traits::{Storage, StorageResult};
///
/// struct PostgresStorage {
///     pool: sqlx::PgPool,
/// }
///
/// #[async_trait]
/// impl Storage for PostgresStorage {
///     async fn health_check(&self) -> StorageResult<()> {
///         // Perform a simple query to verify database connectivity
///         sqlx::query("SELECT 1")
///             .execute(&self.pool)
///             .await
///             .map_err(|e| StorageError::DatabaseError {
///                 details: format!("Health check failed: {}", e),
///             })?;
///         Ok(())
///     }
/// }
/// ```
#[async_trait]
pub trait Storage: Send + Sync {
    /// Performs a health check on the storage backend.
    ///
    /// This method verifies that the storage backend is accessible and functioning
    /// correctly. It should perform a lightweight operation that confirms basic
    /// connectivity and operational status.
    ///
    /// # Returns
    ///
    /// * `Ok(())` - Health check passed, storage is operational
    /// * `Err(StorageError)` - Health check failed, with error details
    ///
    /// # Implementation Notes
    ///
    /// - Keep the health check operation lightweight to avoid performance impact
    /// - For database implementations, a simple `SELECT 1` query is often sufficient
    /// - For Redis implementations, a `PING` command works well
    /// - Include meaningful error information if the check fails
    ///
    /// # Examples
    ///
    /// ```rust,ignore
    /// use ifthisthenat::storage::traits::Storage;
    ///
    /// async fn monitor_storage_health(storage: &dyn Storage) {
    ///     match storage.health_check().await {
    ///         Ok(()) => println!("Storage is healthy"),
    ///         Err(e) => eprintln!("Storage health check failed: {}", e),
    ///     }
    /// }
    /// ```
    async fn health_check(&self) -> StorageResult<()>;
}
