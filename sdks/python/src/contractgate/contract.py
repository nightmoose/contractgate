"""Contract types — pure-Python port of ``src/contract.rs``.

Mirrors the YAML schema accepted by the Rust validator so a contract
authored once parses identically on both sides. Field names match the
Rust ``serde`` rename rules (``type``, ``enum``, ``properties``, etc.).

Compile-once-validate-many: ``Contract.compile()`` returns a
``CompiledContract`` with regex patterns pre-compiled, the declared
top-level field set cached for compliance-mode lookups, and the
transform-types pre-validated.

The compiled object is the entry point to the local validator (see
``validator.py``). The local validator is **read-only**: it reports
violations but does NOT run RFC-004 PII transforms (the gateway is
the single source of truth for the post-transform payload).
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from enum import Enum
from typing import Any, Dict, List, Optional, Pattern, Set

import yaml

from contractgate.exceptions import ContractCompileError


# ---------------------------------------------------------------------------
# Ontology types
# ---------------------------------------------------------------------------


class FieldType(str, Enum):
    """Supported field types inside a contract ontology.

    ``"number"`` is accepted as an alias for ``"float"`` (matches Rust's
    ``#[serde(alias = "number")]``).
    """

    STRING = "string"
    INTEGER = "integer"
    FLOAT = "float"
    BOOLEAN = "boolean"
    OBJECT = "object"
    ARRAY = "array"
    ANY = "any"

    @classmethod
    def from_yaml(cls, raw: Any) -> "FieldType":
        if not isinstance(raw, str):
            raise ContractCompileError(f"field type must be a string, got {type(raw).__name__}")
        normalized = raw.lower()
        if normalized == "number":
            normalized = "float"
        try:
            return cls(normalized)
        except ValueError as e:
            raise ContractCompileError(f"unknown field type: {raw!r}") from e


class TransformKind(str, Enum):
    """RFC-004 transform kinds. Declared in contracts; not run locally."""

    MASK = "mask"
    HASH = "hash"
    DROP = "drop"
    REDACT = "redact"


class MaskStyle(str, Enum):
    """Sub-setting for ``TransformKind.MASK``."""

    OPAQUE = "opaque"
    FORMAT_PRESERVING = "format_preserving"


@dataclass(frozen=True)
class Transform:
    """RFC-004 PII transform declaration.

    The local validator records this for documentation but never
    applies it. The gateway runs transforms in its ingest pipeline.
    """

    kind: TransformKind
    style: Optional[MaskStyle] = None

    @classmethod
    def from_yaml(cls, raw: Any) -> "Transform":
        if not isinstance(raw, dict):
            raise ContractCompileError("transform must be a mapping")
        kind_raw = raw.get("kind")
        if not isinstance(kind_raw, str):
            raise ContractCompileError("transform.kind is required and must be a string")
        try:
            kind = TransformKind(kind_raw.lower())
        except ValueError as e:
            raise ContractCompileError(f"unknown transform kind: {kind_raw!r}") from e
        style: Optional[MaskStyle] = None
        if "style" in raw and raw["style"] is not None:
            try:
                style = MaskStyle(str(raw["style"]).lower())
            except ValueError as e:
                raise ContractCompileError(
                    f"unknown mask style: {raw['style']!r}"
                ) from e
        return cls(kind=kind, style=style)


@dataclass(frozen=True)
class FieldDefinition:
    """One ontology entity (field). Mirrors Rust ``FieldDefinition``."""

    name: str
    field_type: FieldType
    required: bool = True
    pattern: Optional[str] = None
    allowed_values: Optional[List[Any]] = None
    min: Optional[float] = None
    max: Optional[float] = None
    min_length: Optional[int] = None
    max_length: Optional[int] = None
    properties: Optional[List["FieldDefinition"]] = None
    items: Optional["FieldDefinition"] = None
    transform: Optional[Transform] = None

    @classmethod
    def from_yaml(cls, raw: Any) -> "FieldDefinition":
        if not isinstance(raw, dict):
            raise ContractCompileError(f"entity must be a mapping, got {type(raw).__name__}")
        name = raw.get("name")
        if not isinstance(name, str) or not name:
            raise ContractCompileError("entity.name is required")
        ft = FieldType.from_yaml(raw.get("type"))
        properties = None
        if raw.get("properties") is not None:
            if not isinstance(raw["properties"], list):
                raise ContractCompileError(f"{name}.properties must be a list")
            properties = [FieldDefinition.from_yaml(p) for p in raw["properties"]]
        items = None
        if raw.get("items") is not None:
            items = FieldDefinition.from_yaml(raw["items"])
        transform = None
        if raw.get("transform") is not None:
            transform = Transform.from_yaml(raw["transform"])
        return cls(
            name=name,
            field_type=ft,
            required=bool(raw.get("required", True)),
            pattern=_opt_str(raw.get("pattern")),
            allowed_values=raw.get("enum"),
            min=_opt_float(raw.get("min")),
            max=_opt_float(raw.get("max")),
            min_length=_opt_int(raw.get("min_length")),
            max_length=_opt_int(raw.get("max_length")),
            properties=properties,
            items=items,
            transform=transform,
        )


# ---------------------------------------------------------------------------
# Metric / glossary
# ---------------------------------------------------------------------------


class MetricType(str, Enum):
    INTEGER = "integer"
    FLOAT = "float"


@dataclass(frozen=True)
class MetricDefinition:
    name: str
    field: Optional[str] = None
    metric_type: Optional[MetricType] = None
    formula: Optional[str] = None
    min: Optional[float] = None
    max: Optional[float] = None

    @classmethod
    def from_yaml(cls, raw: Any) -> "MetricDefinition":
        if not isinstance(raw, dict):
            raise ContractCompileError("metric must be a mapping")
        name = raw.get("name")
        if not isinstance(name, str) or not name:
            raise ContractCompileError("metric.name is required")
        mt: Optional[MetricType] = None
        if raw.get("type") is not None:
            try:
                mt = MetricType(str(raw["type"]).lower())
            except ValueError as e:
                raise ContractCompileError(
                    f"unknown metric type: {raw['type']!r}"
                ) from e
        return cls(
            name=name,
            field=_opt_str(raw.get("field")),
            metric_type=mt,
            formula=_opt_str(raw.get("formula")),
            min=_opt_float(raw.get("min")),
            max=_opt_float(raw.get("max")),
        )


@dataclass(frozen=True)
class GlossaryEntry:
    field: str
    description: str
    constraints: Optional[str] = None
    synonyms: Optional[List[str]] = None

    @classmethod
    def from_yaml(cls, raw: Any) -> "GlossaryEntry":
        if not isinstance(raw, dict):
            raise ContractCompileError("glossary entry must be a mapping")
        field_name = raw.get("field") or raw.get("term")
        description = raw.get("description") or raw.get("definition")
        if not isinstance(field_name, str) or not field_name:
            raise ContractCompileError("glossary.field (or .term) is required")
        if not isinstance(description, str) or not description:
            raise ContractCompileError("glossary.description (or .definition) is required")
        synonyms = raw.get("synonyms")
        if synonyms is not None and not isinstance(synonyms, list):
            raise ContractCompileError("glossary.synonyms must be a list")
        return cls(
            field=field_name,
            description=description,
            constraints=_opt_str(raw.get("constraints")),
            synonyms=list(synonyms) if synonyms else None,
        )


# ---------------------------------------------------------------------------
# Top-level Contract
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class Contract:
    """A semantic contract — the YAML root."""

    version: str
    name: str
    description: Optional[str]
    compliance_mode: bool
    entities: List[FieldDefinition]
    glossary: List[GlossaryEntry] = field(default_factory=list)
    metrics: List[MetricDefinition] = field(default_factory=list)

    @classmethod
    def from_yaml(cls, source: str) -> "Contract":
        """Parse a contract YAML string into a ``Contract``."""
        try:
            raw = yaml.safe_load(source)
        except yaml.YAMLError as e:
            raise ContractCompileError(f"invalid YAML: {e}") from e
        if not isinstance(raw, dict):
            raise ContractCompileError("contract YAML must be a mapping at the top level")

        version = raw.get("version")
        name = raw.get("name")
        if not isinstance(version, str) or not version:
            raise ContractCompileError("contract.version is required")
        if not isinstance(name, str) or not name:
            raise ContractCompileError("contract.name is required")

        ontology_raw = raw.get("ontology")
        if not isinstance(ontology_raw, dict):
            raise ContractCompileError("contract.ontology is required and must be a mapping")
        entities_raw = ontology_raw.get("entities")
        if not isinstance(entities_raw, list):
            raise ContractCompileError("contract.ontology.entities must be a list")
        entities = [FieldDefinition.from_yaml(e) for e in entities_raw]

        glossary = [GlossaryEntry.from_yaml(g) for g in (raw.get("glossary") or [])]
        metrics = [MetricDefinition.from_yaml(m) for m in (raw.get("metrics") or [])]

        return cls(
            version=version,
            name=name,
            description=_opt_str(raw.get("description")),
            compliance_mode=bool(raw.get("compliance_mode", False)),
            entities=entities,
            glossary=glossary,
            metrics=metrics,
        )

    def compile(self) -> "CompiledContract":
        """Pre-compile regex patterns and validate transform types.

        Raises ``ContractCompileError`` on a bad regex or on a
        transform declared on a non-string field.
        """
        patterns: Dict[str, Pattern[str]] = {}
        _compile_field_patterns(self.entities, "", patterns)
        _validate_transform_types(self.entities, "")

        declared: Set[str] = set()
        if self.compliance_mode:
            declared = {e.name for e in self.entities}

        return CompiledContract(
            contract=self,
            patterns=patterns,
            declared_top_level_fields=declared,
        )


@dataclass(frozen=True)
class CompiledContract:
    """A ``Contract`` with regex patterns pre-compiled and metadata cached.

    Construct via :py:meth:`Contract.compile`. Use :py:meth:`validate`
    to check events.
    """

    contract: Contract
    patterns: Dict[str, Pattern[str]]
    declared_top_level_fields: Set[str]

    def validate(self, event: Any) -> "ValidationResult":  # noqa: F821 — forward ref
        """Validate ``event`` against the compiled contract.

        Always returns a ``ValidationResult`` — never raises.
        """
        # Local import avoids a circular reference between
        # ``contract.py`` (defines ``CompiledContract``) and
        # ``validator.py`` (consumes it). Keeps the public ``validate``
        # callable straight off the compiled object.
        from contractgate.validator import validate as _validate

        return _validate(self, event)


# ---------------------------------------------------------------------------
# Compile-stage helpers
# ---------------------------------------------------------------------------


def _compile_field_patterns(
    fields: List[FieldDefinition],
    prefix: str,
    out: Dict[str, Pattern[str]],
) -> None:
    """Recursively walk and pre-compile every ``pattern`` regex."""
    for f in fields:
        path = f.name if not prefix else f"{prefix}.{f.name}"
        if f.pattern is not None:
            try:
                out[path] = re.compile(f.pattern)
            except re.error as e:
                raise ContractCompileError(
                    f"Invalid regex {f.pattern!r} for field {path!r}: {e}"
                ) from e
        if f.field_type == FieldType.OBJECT and f.properties:
            _compile_field_patterns(f.properties, path, out)


def _validate_transform_types(fields: List[FieldDefinition], prefix: str) -> None:
    """Reject transforms on non-string fields. Same wording as Rust."""
    for f in fields:
        path = f.name if not prefix else f"{prefix}.{f.name}"
        if f.transform is not None and f.field_type != FieldType.STRING:
            raise ContractCompileError(
                f"Field '{path}' declares a PII transform but has type "
                f"'{_rust_field_type_repr(f.field_type)}' — transforms are only "
                "supported on string fields. If this field holds PII, change "
                "its type to 'string'."
            )
        if f.field_type == FieldType.OBJECT and f.properties:
            _validate_transform_types(f.properties, path)


def _rust_field_type_repr(ft: FieldType) -> str:
    """Match the Rust ``Debug`` rendering of ``FieldType`` (e.g. ``Integer``).

    The Rust error message is built with ``{:?}`` which formats the enum
    variant in PascalCase. We mirror that exactly so users see one
    canonical error string regardless of which side caught the bad
    contract.
    """
    return {
        FieldType.STRING: "String",
        FieldType.INTEGER: "Integer",
        FieldType.FLOAT: "Float",
        FieldType.BOOLEAN: "Boolean",
        FieldType.OBJECT: "Object",
        FieldType.ARRAY: "Array",
        FieldType.ANY: "Any",
    }[ft]


# ---------------------------------------------------------------------------
# Coercion helpers
# ---------------------------------------------------------------------------


def _opt_str(v: Any) -> Optional[str]:
    if v is None:
        return None
    if isinstance(v, str):
        return v
    raise ContractCompileError(f"expected string, got {type(v).__name__}")


def _opt_int(v: Any) -> Optional[int]:
    if v is None:
        return None
    if isinstance(v, bool):  # bool is an int subclass — reject explicitly
        raise ContractCompileError("expected integer, got boolean")
    if isinstance(v, int):
        return v
    raise ContractCompileError(f"expected integer, got {type(v).__name__}")


def _opt_float(v: Any) -> Optional[float]:
    if v is None:
        return None
    if isinstance(v, bool):
        raise ContractCompileError("expected number, got boolean")
    if isinstance(v, (int, float)):
        return float(v)
    raise ContractCompileError(f"expected number, got {type(v).__name__}")
