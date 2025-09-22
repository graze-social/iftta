# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.0.0-rc.1] - 2025-09-21

### Added
- Initial release candidate of ifthisthenat AT Protocol automation service
- Blueprint-based automation system with ordered node evaluation
- Jetstream event consumer with partitioning support
- Webhook entry nodes for HTTP trigger support
- Periodic entry nodes with cron schedule support
- Condition nodes for boolean flow control using DataLogic
- Transform nodes for data manipulation using DataLogic
- Publish record action for creating AT Protocol records
- Publish webhook action for sending webhook notifications
- Facet text node for processing text with mentions and facets
- Debug action node for development logging
- Redis support for caching and distributed queues
- PostgreSQL storage layer for persistent data
- OAuth authentication with AT Protocol
- XRPC API endpoints for blueprint management
- Multi-worker thread processing for scalability
- Blueprint cache with LRU eviction
- Comprehensive error handling system
- Metrics collection and export support
- Docker Compose development environment
- SQLx database migrations

### Security
- Cookie-based session management with encryption
- DID-based authentication and authorization
- Rate limiting and throttling support

[Unreleased]: https://github.com/graze-social/iftta/compare/v1.0.0-rc.1...HEAD
[1.0.0-rc.1]: https://github.com/graze-social/iftta/compare/9558d9d7ba728cce75c5b395101ccd5b1cbf05bd...v1.0.0-rc.1