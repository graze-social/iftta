# Contributing to ifthisthenat

Thank you for your interest in contributing to ifthisthenat! This guide will help you get started with contributing to the project.

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Project Structure](#project-structure)
- [Making Changes](#making-changes)
- [Testing](#testing)
- [Documentation](#documentation)
- [Submitting Changes](#submitting-changes)
- [Review Process](#review-process)
- [Release Process](#release-process)

## Code of Conduct

This project follows the [Rust Code of Conduct](https://www.rust-lang.org/policies/code-of-conduct). Please read and follow it in all your interactions with the project.

## Getting Started

### Prerequisites

- **Rust**: 1.70+ (latest stable recommended)
- **PostgreSQL**: 13+ for local development
- **Redis**: 6+ (optional, for advanced features)
- **Docker**: For containerized development and testing
- **Git**: For version control

### Quick Setup

1. **Fork and clone the repository:**
   ```bash
   git clone https://github.com/your-username/ifthisthenat.git
   cd ifthisthenat
   ```

2. **Set up the development environment:**
   ```bash
   # Install Rust dependencies
   cargo build

   # Start development services
   docker-compose up -d postgres redis

   # Copy and configure environment
   cp .env.example .env
   # Edit .env with your development settings
   ```

3. **Run the application:**
   ```bash
   cargo run
   ```

4. **Run tests:**
   ```bash
   cargo test
   ```

## Development Setup

### Environment Configuration

Copy the example environment file and configure for development:

```bash
cp .env.example .env
```

Key development settings:
```bash
# Database
DATABASE_URL=postgres://ifthisthenat:password@localhost:5432/ifthisthenat_dev

# Basic configuration
EXTERNAL_BASE=http://localhost:8080
HTTP_COOKIE_KEY=development-cookie-key-not-for-production-use
ISSUER_DID=did:plc:development

# Enable debug logging
RUST_LOG=ifthisthenat=debug,info
RUST_BACKTRACE=1

# Development OAuth (mock or test credentials)
AIP_HOSTNAME=https://test.example.com
AIP_CLIENT_ID=test-client-id
AIP_CLIENT_SECRET=test-client-secret
```

### Database Setup

The project uses SQLx for database migrations:

```bash
# Install sqlx-cli if not already installed
cargo install sqlx-cli --no-default-features --features postgres

# Run migrations
sqlx migrate run

# Create a new migration (if needed)
sqlx migrate add create_new_table
```

### Redis Setup (Optional)

For testing Redis-dependent features:

```bash
# Start Redis
docker run -d --name redis -p 6379:6379 redis:alpine

# Add to .env
REDIS_URL=redis://localhost:6379
```

## Project Structure

```
ifthisthenat/
├── src/
│   ├── bin/               # Binary entrypoints
│   ├── consumer.rs        # Jetstream event consumer
│   ├── engine/            # Blueprint evaluation engine
│   ├── storage/           # Data storage abstractions
│   ├── tasks/             # Background task processors
│   ├── http.rs            # HTTP server and API endpoints
│   └── lib.rs             # Library root
├── migrations/            # Database schema migrations
├── charts/                # Helm charts for Kubernetes
├── docs/                  # Documentation
├── tests/                 # Integration tests
└── examples/              # Example configurations
```

### Key Components

- **Engine** (`src/engine/`): Blueprint evaluation logic and node types
- **Storage** (`src/storage/`): Database abstractions and implementations
- **Tasks** (`src/tasks/`): Background processing for webhooks, scheduling, etc.
- **Consumer** (`src/consumer.rs`): AT Protocol Jetstream event processing
- **HTTP** (`src/http.rs`): Web server, API endpoints, and webhook handlers

## Making Changes

### Branch Naming

Use descriptive branch names:
- `feature/add-new-node-type`
- `fix/blueprint-evaluation-bug`
- `docs/update-configuration-guide`
- `refactor/storage-layer-cleanup`

### Commit Messages

Follow conventional commit format:
```
type(scope): description

[optional body]

[optional footer]
```

Types:
- `feat`: New feature
- `fix`: Bug fix
- `docs`: Documentation changes
- `style`: Formatting, missing semicolons, etc.
- `refactor`: Code refactoring
- `test`: Adding or updating tests
- `chore`: Maintenance tasks

Examples:
```
feat(engine): add periodic_entry node type

Add support for cron-based blueprint triggers with validation
for minimum 30-second intervals and maximum 90-day intervals.

Closes #123

fix(webhook): handle timeout errors gracefully

Previously, webhook timeouts would cause blueprint evaluation
to fail. Now timeouts are logged and the blueprint continues.

docs(config): update environment variable documentation

Add missing Redis configuration options and clarify OAuth
scope requirements for different node types.
```

### Code Style

The project uses `rustfmt` and `clippy` for code formatting and linting:

```bash
# Format code
cargo fmt

# Run linter
cargo clippy -- -D warnings

# Run both as part of pre-commit checks
cargo fmt --check && cargo clippy -- -D warnings
```

### Error Handling

Follow the project's error handling patterns:

1. **Use structured errors** with `thiserror`:
   ```rust
   #[derive(thiserror::Error, Debug)]
   pub enum MyError {
       #[error("error-iftta-mydomain-1 Invalid configuration: {field}")]
       InvalidConfig { field: String },

       #[error("error-iftta-mydomain-2 Database operation failed: {source}")]
       DatabaseError { source: sqlx::Error },
   }
   ```

2. **Error format**: `error-iftta-<domain>-<number> <message>: <details>`

3. **Avoid `anyhow!()` for new errors** - create proper error types

4. **Log errors appropriately**:
   ```rust
   match operation().await {
       Ok(result) => info!("Operation succeeded"),
       Err(e) => error!("Operation failed: {}", e),
   }
   ```

### Adding New Node Types

To add a new node type:

1. **Define the node type**:
   ```rust
   // src/engine/node_type_my_new_type.rs
   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct MyNewTypeConfiguration {
       pub field: String,
   }

   #[async_trait]
   impl NodeTypeEvaluator for MyNewTypeEvaluator {
       async fn evaluate(&self, input: Value, config: Value) -> Result<Value, NodeEvaluationError> {
           // Implementation
       }
   }
   ```

2. **Register in the factory**:
   ```rust
   // src/engine/mod.rs
   pub fn create_node_evaluator(node_type: &str) -> Result<Box<dyn NodeTypeEvaluator>, Error> {
       match node_type {
           "my_new_type" => Ok(Box::new(MyNewTypeEvaluator::new())),
           // ... other types
       }
   }
   ```

3. **Add tests**:
   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;

       #[tokio::test]
       async fn test_my_new_type_evaluation() {
           // Test implementation
       }
   }
   ```

4. **Update documentation** in `docs/blueprints.md`

## Testing

### Running Tests

```bash
# Run all tests
cargo test

# Run specific test
cargo test test_name

# Run tests with output
cargo test -- --nocapture

# Run integration tests only
cargo test --test integration

# Run with test database
TEST_DATABASE_URL=postgres://localhost/ifthisthenat_test cargo test
```

### Test Categories

1. **Unit Tests**: Test individual functions and components
2. **Integration Tests**: Test component interactions
3. **End-to-End Tests**: Test complete workflows

### Writing Tests

Example unit test:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_blueprint_evaluation() {
        let blueprint = Blueprint {
            nodes: vec![/* test nodes */],
        };

        let result = evaluate_blueprint(blueprint).await;
        assert!(result.is_ok());
    }
}
```

Example integration test:
```rust
// tests/integration_test.rs
use ifthisthenat::storage::postgres::PostgresStorage;

#[tokio::test]
async fn test_blueprint_storage() {
    let storage = PostgresStorage::new(&test_database_url()).await.unwrap();

    // Test blueprint CRUD operations
}
```

### Test Database

For tests requiring a database:

```bash
# Create test database
createdb ifthisthenat_test

# Run migrations
TEST_DATABASE_URL=postgres://localhost/ifthisthenat_test sqlx migrate run

# Run tests
TEST_DATABASE_URL=postgres://localhost/ifthisthenat_test cargo test
```

## Documentation

### Types of Documentation

1. **Code Documentation**: Rustdoc comments
2. **User Guides**: Markdown files in `docs/`
3. **API Documentation**: Auto-generated from code
4. **Examples**: Working example configurations

### Writing Documentation

1. **Rustdoc Comments**:
   ```rust
   /// Evaluates a blueprint with the given input data.
   ///
   /// # Arguments
   ///
   /// * `blueprint` - The blueprint to evaluate
   /// * `input` - Input data for evaluation
   ///
   /// # Returns
   ///
   /// Returns the evaluation result or an error if evaluation fails.
   ///
   /// # Example
   ///
   /// ```
   /// let result = evaluate_blueprint(blueprint, input).await?;
   /// ```
   pub async fn evaluate_blueprint(blueprint: Blueprint, input: Value) -> Result<Value, Error> {
       // Implementation
   }
   ```

2. **User Documentation**: Update relevant files in `docs/`
   - `docs/configuration.md` - Configuration options
   - `docs/blueprints.md` - Blueprint creation and node types
   - `docs/deployment-standalone.md` - Deployment guides
   - `docs/storage-and-queues.md` - Storage and queue options

3. **Example Configurations**: Add to `examples/` directory

### Building Documentation

```bash
# Build Rustdoc documentation
cargo doc --open

# Check documentation links
cargo doc --no-deps
```

## Submitting Changes

### Pull Request Process

1. **Create a feature branch**:
   ```bash
   git checkout -b feature/my-new-feature
   ```

2. **Make your changes** following the guidelines above

3. **Test your changes**:
   ```bash
   cargo test
   cargo fmt --check
   cargo clippy -- -D warnings
   ```

4. **Commit your changes**:
   ```bash
   git add .
   git commit -m "feat(engine): add new node type for X"
   ```

5. **Push to your fork**:
   ```bash
   git push origin feature/my-new-feature
   ```

6. **Create a pull request** with:
   - Clear title and description
   - Reference to related issues
   - Description of changes made
   - Testing performed

### Pull Request Template

```markdown
## Summary

Brief description of changes.

## Related Issues

Fixes #123

## Changes Made

- Added new feature X
- Fixed bug in Y
- Updated documentation for Z

## Testing

- [ ] Unit tests pass
- [ ] Integration tests pass
- [ ] Manual testing performed
- [ ] Documentation updated

## Breaking Changes

None / List any breaking changes

## Additional Notes

Any additional context or notes for reviewers.
```

## Review Process

### Review Criteria

Pull requests are reviewed for:

1. **Functionality**: Does it work as intended?
2. **Code Quality**: Is the code well-written and maintainable?
3. **Testing**: Are there adequate tests?
4. **Documentation**: Is documentation updated/added?
5. **Performance**: Are there performance implications?
6. **Security**: Are there security considerations?
7. **Breaking Changes**: Are breaking changes documented?

### Reviewer Guidelines

When reviewing:

1. **Be constructive** and provide helpful feedback
2. **Ask questions** if something is unclear
3. **Suggest improvements** rather than just pointing out problems
4. **Approve** when the PR meets quality standards
5. **Test** the changes if possible

### Addressing Feedback

When receiving feedback:

1. **Respond promptly** to questions and suggestions
2. **Make requested changes** or explain why they're not needed
3. **Test changes** after making updates
4. **Re-request review** after addressing feedback

## Release Process

### Versioning

The project follows [Semantic Versioning](https://semver.org/):
- **MAJOR**: Breaking changes
- **MINOR**: New features (backward compatible)
- **PATCH**: Bug fixes (backward compatible)

### Release Workflow

1. **Update version** in `Cargo.toml`
2. **Update CHANGELOG.md** with release notes
3. **Create release commit**: `chore: release v1.2.3`
4. **Tag the release**: `git tag v1.2.3`
5. **Push tag**: `git push origin v1.2.3`
6. **Create GitHub release** with release notes

### Release Notes Format

```markdown
## [1.2.3] - 2024-12-01

### Added
- New periodic_entry node type for scheduled blueprints
- Redis cursor storage support

### Changed
- Improved error messages for configuration validation
- Updated dependencies

### Fixed
- Fixed blueprint evaluation timeout handling
- Resolved webhook retry logic issue

### Security
- Updated authentication token validation

### Breaking Changes
- Changed webhook configuration format (see migration guide)
```

## Getting Help

- **Issues**: Create GitHub issues for bugs and feature requests
- **Discussions**: Use GitHub Discussions for questions and ideas
- **Documentation**: Check the `docs/` directory for guides
- **Code Examples**: Look in the `examples/` directory

## Recognition

Contributors are recognized in:
- CHANGELOG.md release notes
- GitHub contributor graphs
- Special recognition for significant contributions

Thank you for contributing to ifthisthenat! 🎉