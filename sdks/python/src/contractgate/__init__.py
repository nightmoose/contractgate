"""ContractGate Python SDK.

First-party client and pure-Python validator for the ContractGate
semantic contract enforcement gateway.

Public surface:
    Client, AsyncClient            -- HTTP clients (sync, async)
    Contract, CompiledContract     -- local contract parse + compile
    FieldDefinition, FieldType     -- ontology types
    MetricDefinition, MetricType   -- metric types
    Transform, TransformKind,      -- RFC-004 declarations (declared, not run)
        MaskStyle
    ValidationResult, Violation,   -- validator outputs
        ViolationKind
    BatchIngestResponse,           -- HTTP response shapes
        IngestEventResult,
        AuditEntry, ContractResponse,
        VersionResponse, VersionSummary,
        IngestionStats
    ContractGateError, HTTPError,  -- error hierarchy
        BadRequestError, AuthError,
        NotFoundError, ConflictError,
        ValidationFailedError,
        ServerError, ConnectionError,
        ContractCompileError

See README.md for usage. See ../docs/rfcs/005-python-sdk.md for the
design rationale.
"""

from contractgate._version import __version__
from contractgate.async_client import AsyncClient
from contractgate.client import Client
from contractgate.contract import (
    CompiledContract,
    Contract,
    FieldDefinition,
    FieldType,
    MaskStyle,
    MetricDefinition,
    MetricType,
    Transform,
    TransformKind,
)
from contractgate.exceptions import (
    AuthError,
    BadRequestError,
    ConflictError,
    ConnectionError,
    ContractCompileError,
    ContractGateError,
    HTTPError,
    NotFoundError,
    ServerError,
    ValidationFailedError,
)
from contractgate.models import (
    AuditEntry,
    BatchIngestResponse,
    ContractResponse,
    IngestEventResult,
    IngestionStats,
    ValidationResult,
    VersionResponse,
    VersionSummary,
    Violation,
    ViolationKind,
)

__all__ = [
    "__version__",
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
]
