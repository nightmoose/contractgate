"""AISchemaGate — alias over ``contractgate`` for LLM output validation.

RFC-094 §4. This package has no behavior of its own. It re-exports the
`contractgate` public surface and the `contractgate.llm` additions so
`from aischemagate import LLMGate` works for AISchemaGate users without
them typing `contractgate.llm` in their imports. Breaking changes are
made in `contractgate`; this alias follows via its minimum-version pin.

Both brands share the same backend, tenant service, and API keys. The
`api_key` is the same `cg_live_...` the dashboard issues.
"""

from contractgate import (
    AsyncClient,
    AuditEntry,
    AuthError,
    BadRequestError,
    BatchIngestResponse,
    Client,
    CompiledContract,
    ConflictError,
    ConnectionError,
    Contract,
    ContractCompileError,
    ContractGateError,
    ContractResponse,
    EgressOutcome,
    EgressResponse,
    FieldDefinition,
    FieldType,
    HTTPError,
    IngestEventResult,
    IngestionStats,
    MaskStyle,
    MetricDefinition,
    MetricType,
    NotFoundError,
    ServerError,
    Transform,
    TransformKind,
    ValidationFailedError,
    ValidationResult,
    VersionResponse,
    VersionSummary,
    Violation,
    ViolationKind,
    __version__ as _contractgate_version,
)
from contractgate.llm import (
    MAX_RETRIES_CAP,
    AsyncLLMGate,
    LLMGate,
    LLMGateResult,
    LLMResponseError,
    extract_json,
)

__version__ = "0.3.1"
__contractgate_version__ = _contractgate_version

__all__ = [
    "__version__",
    "__contractgate_version__",
    # Clients
    "Client",
    "AsyncClient",
    # Contract / validator
    "Contract",
    "CompiledContract",
    "FieldDefinition",
    "FieldType",
    "MetricDefinition",
    "MetricType",
    "Transform",
    "TransformKind",
    "MaskStyle",
    # Models
    "ValidationResult",
    "Violation",
    "ViolationKind",
    "BatchIngestResponse",
    "IngestEventResult",
    "EgressOutcome",
    "EgressResponse",
    "AuditEntry",
    "ContractResponse",
    "VersionResponse",
    "VersionSummary",
    "IngestionStats",
    # Errors
    "ContractGateError",
    "HTTPError",
    "BadRequestError",
    "AuthError",
    "NotFoundError",
    "ConflictError",
    "ValidationFailedError",
    "ServerError",
    "ConnectionError",
    "ContractCompileError",
    # LLM (RFC-094)
    "LLMGate",
    "AsyncLLMGate",
    "LLMGateResult",
    "LLMResponseError",
    "extract_json",
    "MAX_RETRIES_CAP",
]
