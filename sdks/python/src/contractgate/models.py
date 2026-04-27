"""Wire-shape response models.

Frozen dataclasses that mirror the JSON the gateway returns. Field
names match the wire format exactly (snake_case both sides — no
rename layer).

A ``ValidationResult`` is also produced by the local validator, so it
must match the Rust ``ValidationResult`` JSON shape so callers can
move between local and server-side validation results without
remapping fields.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Dict, List, Optional


class ViolationKind(str, Enum):
    """Mirrors Rust ``ViolationKind`` (``serde(rename_all = "snake_case")``)."""

    MISSING_REQUIRED_FIELD = "missing_required_field"
    TYPE_MISMATCH = "type_mismatch"
    PATTERN_MISMATCH = "pattern_mismatch"
    ENUM_VIOLATION = "enum_violation"
    RANGE_VIOLATION = "range_violation"
    LENGTH_VIOLATION = "length_violation"
    METRIC_RANGE_VIOLATION = "metric_range_violation"
    UNKNOWN_FIELD = "unknown_field"
    UNDECLARED_FIELD = "undeclared_field"


@dataclass(frozen=True)
class Violation:
    field: str
    message: str
    kind: ViolationKind

    @classmethod
    def from_dict(cls, raw: Dict[str, Any]) -> "Violation":
        return cls(
            field=raw["field"],
            message=raw["message"],
            kind=ViolationKind(raw["kind"]),
        )


@dataclass(frozen=True)
class ValidationResult:
    passed: bool
    violations: List[Violation] = field(default_factory=list)
    validation_us: int = 0

    @classmethod
    def from_dict(cls, raw: Dict[str, Any]) -> "ValidationResult":
        return cls(
            passed=bool(raw.get("passed", False)),
            violations=[Violation.from_dict(v) for v in raw.get("violations", [])],
            validation_us=int(raw.get("validation_us", 0)),
        )


# ---------------------------------------------------------------------------
# Ingest response
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class IngestEventResult:
    """Per-event result inside a ``BatchIngestResponse``.

    ``contract_version`` is the version that *actually* matched the
    event (relevant under ``multi_stable_resolution: fallback``). The
    audit-honesty rule says: surface this as-is, never substitute the
    requested version.

    ``transformed_event`` is the post-transform payload that the
    gateway wrote to durable storage and forwarded downstream
    (RFC-004). When the matching contract declares no transforms it
    is byte-for-byte identical to the request body.
    """

    passed: bool
    violations: List[Violation]
    validation_us: int
    forwarded: bool
    contract_version: str
    transformed_event: Any

    @classmethod
    def from_dict(cls, raw: Dict[str, Any]) -> "IngestEventResult":
        return cls(
            passed=bool(raw["passed"]),
            violations=[Violation.from_dict(v) for v in raw.get("violations", [])],
            validation_us=int(raw.get("validation_us", 0)),
            forwarded=bool(raw.get("forwarded", False)),
            contract_version=raw["contract_version"],
            transformed_event=raw.get("transformed_event"),
        )


@dataclass(frozen=True)
class BatchIngestResponse:
    total: int
    passed: int
    failed: int
    dry_run: bool
    atomic: bool
    resolved_version: str
    version_pin_source: str
    results: List[IngestEventResult]

    @classmethod
    def from_dict(cls, raw: Dict[str, Any]) -> "BatchIngestResponse":
        return cls(
            total=int(raw["total"]),
            passed=int(raw["passed"]),
            failed=int(raw["failed"]),
            dry_run=bool(raw.get("dry_run", False)),
            atomic=bool(raw.get("atomic", False)),
            resolved_version=raw["resolved_version"],
            version_pin_source=raw["version_pin_source"],
            results=[IngestEventResult.from_dict(r) for r in raw.get("results", [])],
        )


# ---------------------------------------------------------------------------
# Contract / version response shapes
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class ContractResponse:
    id: str
    name: str
    description: Optional[str]
    multi_stable_resolution: str
    created_at: str
    updated_at: str
    version_count: int
    latest_stable_version: Optional[str]

    @classmethod
    def from_dict(cls, raw: Dict[str, Any]) -> "ContractResponse":
        return cls(
            id=raw["id"],
            name=raw["name"],
            description=raw.get("description"),
            multi_stable_resolution=raw.get("multi_stable_resolution", "strict"),
            created_at=raw["created_at"],
            updated_at=raw["updated_at"],
            version_count=int(raw.get("version_count", 0)),
            latest_stable_version=raw.get("latest_stable_version"),
        )


@dataclass(frozen=True)
class VersionResponse:
    id: str
    contract_id: str
    version: str
    state: str
    yaml_content: str
    created_at: str
    promoted_at: Optional[str]
    deprecated_at: Optional[str]
    compliance_mode: bool

    @classmethod
    def from_dict(cls, raw: Dict[str, Any]) -> "VersionResponse":
        return cls(
            id=raw["id"],
            contract_id=raw["contract_id"],
            version=raw["version"],
            state=raw["state"],
            yaml_content=raw["yaml_content"],
            created_at=raw["created_at"],
            promoted_at=raw.get("promoted_at"),
            deprecated_at=raw.get("deprecated_at"),
            compliance_mode=bool(raw.get("compliance_mode", False)),
        )


@dataclass(frozen=True)
class VersionSummary:
    version: str
    state: str
    created_at: str
    promoted_at: Optional[str]
    deprecated_at: Optional[str]

    @classmethod
    def from_dict(cls, raw: Dict[str, Any]) -> "VersionSummary":
        return cls(
            version=raw["version"],
            state=raw["state"],
            created_at=raw["created_at"],
            promoted_at=raw.get("promoted_at"),
            deprecated_at=raw.get("deprecated_at"),
        )


# ---------------------------------------------------------------------------
# Audit
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class AuditEntry:
    id: str
    contract_id: str
    contract_version: Optional[str]
    passed: bool
    violation_count: int
    violation_details: Any
    raw_event: Any
    validation_us: int
    source_ip: Optional[str]
    created_at: str

    @classmethod
    def from_dict(cls, raw: Dict[str, Any]) -> "AuditEntry":
        return cls(
            id=raw["id"],
            contract_id=raw["contract_id"],
            contract_version=raw.get("contract_version"),
            passed=bool(raw["passed"]),
            violation_count=int(raw.get("violation_count", 0)),
            violation_details=raw.get("violation_details"),
            raw_event=raw.get("raw_event"),
            validation_us=int(raw.get("validation_us", 0)),
            source_ip=raw.get("source_ip"),
            created_at=raw["created_at"],
        )


@dataclass(frozen=True)
class IngestionStats:
    total_events: int
    passed_events: int
    failed_events: int
    pass_rate: float
    avg_validation_us: float
    p50_validation_us: int
    p95_validation_us: int
    p99_validation_us: int

    @classmethod
    def from_dict(cls, raw: Dict[str, Any]) -> "IngestionStats":
        return cls(
            total_events=int(raw.get("total_events", 0)),
            passed_events=int(raw.get("passed_events", 0)),
            failed_events=int(raw.get("failed_events", 0)),
            pass_rate=float(raw.get("pass_rate", 0.0)),
            avg_validation_us=float(raw.get("avg_validation_us", 0.0)),
            p50_validation_us=int(raw.get("p50_validation_us", 0)),
            p95_validation_us=int(raw.get("p95_validation_us", 0)),
            p99_validation_us=int(raw.get("p99_validation_us", 0)),
        )
