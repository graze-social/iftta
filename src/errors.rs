use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("error-iftta-config-1 Required environment variable not set: {var_name}")]
    EnvVarRequired { var_name: String },

    #[error("error-iftta-config-2 Version not available")]
    VersionNotAvailable,

    #[error("error-iftta-config-3 Invalid port number: {port}")]
    InvalidPortNumber { port: String },

    #[error("error-iftta-config-4 Invalid key format: {details}")]
    InvalidKeyFormat { details: String },

    #[error("error-iftta-config-5 Invalid timeout value: {value}")]
    InvalidTimeout { value: String },

    #[error("error-iftta-config-6 Invalid DID: {did}")]
    InvalidDid { did: String },
}

#[derive(Error, Debug)]
pub enum ConsumerError {
    #[error("error-iftta-consumer-2 Invalid partition configuration: {details}")]
    InvalidPartition { details: String },

    #[error("error-iftta-consumer-3 Partition index out of range: {index} >= {total}")]
    PartitionOutOfRange { index: u32, total: u32 },

    #[error("error-iftta-consumer-4 Invalid partition index: {value}")]
    InvalidPartitionIndex { value: String },

    #[error("error-iftta-consumer-5 Invalid partition total: {value}")]
    InvalidPartitionTotal { value: String },

    #[error("error-iftta-consumer-6 Partition total must be greater than 0")]
    PartitionTotalZero,
}

#[derive(Error, Debug)]
pub enum EngineError {
    #[error("error-iftta-engine-1 Node evaluation failed: {node_type} at {node_id}: {details}")]
    NodeEvaluationFailed {
        node_type: String,
        node_id: String,
        details: String,
    },

    #[error("error-iftta-engine-2 DataLogic expression evaluation failed: {expression}: {details}")]
    DataLogicFailed { expression: String, details: String },

    #[error("error-iftta-engine-3 Node configuration invalid: {node_type}: {details}")]
    InvalidNodeConfiguration { node_type: String, details: String },

    #[error("error-iftta-engine-4 Node factory creation failed: {node_type}: {details}")]
    NodeFactoryFailed { node_type: String, details: String },

    #[error("error-iftta-engine-6 Missing required field: {field_name} in {node_type}")]
    MissingRequiredField {
        field_name: String,
        node_type: String,
    },

    #[error(
        "error-iftta-engine-7 Invalid field type: {field_name} in {node_type}, expected {expected_type}"
    )]
    InvalidFieldType {
        field_name: String,
        node_type: String,
        expected_type: String,
    },

    #[error("error-iftta-engine-9 AT Protocol operation failed: {operation}: {details}")]
    AtProtoOperationFailed { operation: String, details: String },

    #[error("error-iftta-engine-10 Record parsing failed: {details}")]
    RecordParsingFailed { details: String },

    #[error("error-iftta-engine-12 Sentiment analysis failed: {details}")]
    SentimentAnalysisFailed { details: String },

    #[error("error-iftta-engine-14 AT-URI parsing failed: {uri}: {details}")]
    AtUriParsingFailed { uri: String, details: String },
}

#[derive(Error, Debug)]
pub enum TaskError {
    #[error("error-iftta-task-10 Blueprint adapter initialization failed: {details}")]
    BlueprintAdapterInitFailed { details: String },
}

#[derive(Error, Debug)]
pub enum ProcessorError {
    #[error("error-iftta-processor-4 Blueprint cache reload failed: {details}")]
    CacheReloadFailed { details: String },
}

#[derive(Error, Debug)]
pub enum QueueError {
    #[error("error-iftta-queue-2 Redis queue operation failed: {operation}: {source}")]
    RedisOperationFailed {
        operation: String,
        #[source]
        source: deadpool_redis::redis::RedisError,
    },

    #[error("error-iftta-queue-3 MPSC queue operation failed: {operation}: {details}")]
    MpscOperationFailed { operation: String, details: String },

    #[error("error-iftta-queue-6 Queue connection failed: {queue_type}: {details}")]
    ConnectionFailed { queue_type: String, details: String },

    #[error("error-iftta-queue-8 Queue capacity exceeded: {queue_type}: {capacity}")]
    CapacityExceeded { queue_type: String, capacity: usize },
}

#[derive(Error, Debug)]
pub enum SerializationError {
    #[error("error-iftta-serialization-1 JSON serialization failed: {data_type}: {source}")]
    JsonSerializationFailed {
        data_type: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("error-iftta-serialization-2 JSON deserialization failed: {data_type}: {source}")]
    JsonDeserializationFailed {
        data_type: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("error-iftta-serialization-3 Binary serialization failed: {data_type}: {details}")]
    BinarySerializationFailed { data_type: String, details: String },

    #[error("error-iftta-serialization-4 Binary deserialization failed: {data_type}: {details}")]
    BinaryDeserializationFailed { data_type: String, details: String },
}

#[derive(Error, Debug)]
pub enum ValidationError {
    #[error("error-iftta-validation-1 Blueprint validation failed: {blueprint_id}: {details}")]
    BlueprintValidationFailed {
        blueprint_id: String,
        details: String,
    },

    #[error("error-iftta-validation-2 Node validation failed: {node_id}: {details}")]
    NodeValidationFailed { node_id: String, details: String },

    #[error("error-iftta-validation-3 Invalid node type: {node_type}")]
    InvalidNodeType { node_type: String },

    #[error("error-iftta-validation-4 Missing required field: {field_name} in {context}")]
    MissingRequiredField { field_name: String, context: String },

    #[error(
        "error-iftta-validation-5 Invalid field type: {field_name} in {context}, expected {expected_type}"
    )]
    InvalidFieldType {
        field_name: String,
        context: String,
        expected_type: String,
    },

    #[error("error-iftta-validation-6 Invalid URL format: {url}: {details}")]
    InvalidUrlFormat { url: String, details: String },

    #[error("error-iftta-validation-7 Entry nodes can only be the first node")]
    EntryNodeNotFirst,

    #[error("error-iftta-validation-8 Blueprint structure invalid: {details}")]
    InvalidBlueprintStructure { details: String },

    #[error("error-iftta-validation-9 Node dependency cycle detected: {cycle_path}")]
    NodeDependencyCycle { cycle_path: String },

    #[error("error-iftta-validation-10 Invalid configuration for {node_type}: {details}")]
    InvalidNodeConfiguration { node_type: String, details: String },

    #[error("error-iftta-validation-11 Transform payload validation failed: {details}")]
    TransformPayloadInvalid { details: String },

    #[error("error-iftta-validation-12 Headers validation failed: {details}")]
    InvalidHeaders { details: String },

    #[error(
        "error-iftta-validation-13 Data type validation failed: {field_name}: expected {expected}, got {actual}"
    )]
    DataTypeValidationFailed {
        field_name: String,
        expected: String,
        actual: String,
    },
}

#[derive(Error, Debug)]
pub enum AuthError {
    #[error("error-iftta-auth-1 Authentication failed: {details}")]
    AuthenticationFailed { details: String },

    #[error("error-iftta-auth-3 Token validation failed: {details}")]
    TokenValidationFailed { details: String },

    #[error("error-iftta-auth-6 OAuth operation failed: {operation}: {details}")]
    OAuthOperationFailed { operation: String, details: String },
}

#[derive(Error, Debug)]
pub enum IdentityError {
    #[error("error-iftta-identity-4 Handle resolution failed: {handle}: {details}")]
    HandleResolutionFailed { handle: String, details: String },
}

#[derive(Error, Debug)]
pub enum LeadershipError {
    #[error("error-iftta-leadership-4 Leadership configuration invalid: {details}")]
    InvalidConfiguration { details: String },
}

#[derive(Debug, Error)]
pub enum HttpError {
    #[error("error-iftta-http-100 Unhandled web error: {details}")]
    Unhandled { details: String },

    #[error("error-iftta-http-101 Request validation failed: {details}")]
    RequestValidation { details: String },

    #[error("error-iftta-http-104 Resource not found: {details}")]
    NotFound { details: String },

    #[error("error-iftta-http-105 Unauthorized: {details}")]
    Unauthorized { details: String },

    #[error("error-iftta-http-106 Bad request: {details}")]
    BadRequest { details: String },

    #[error("error-iftta-http-107 Forbidden: {details}")]
    Forbidden { details: String },
}

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("error-iftta-storage-200 Database connection failed: {source}")]
    ConnectionFailed {
        #[source]
        source: sqlx::Error,
    },

    #[error("error-iftta-storage-201 Transaction failed: {source}")]
    TransactionFailed {
        #[source]
        source: sqlx::Error,
    },

    #[error("error-iftta-storage-202 Query execution failed: {source}")]
    QueryFailed {
        #[source]
        source: sqlx::Error,
    },

    #[error("error-iftta-storage-204 Invalid input data: {details}")]
    InvalidInput { details: String },
}
