"""OpenMetadata -> DCG contract sync (ODCS v3.1.0)."""

from __future__ import annotations

import re
from typing import Optional

from .dcg import DcgClient
from .om import OmClient


def om_to_dcg(om: OmClient, dcg: DcgClient, om_contract_id: str,
              target_contract_id: Optional[str] = None,
              name_override: Optional[str] = None) -> dict:
    """GET the OM contract as ODCS YAML and import it into DCG as a draft version."""
    odcs = om.get_contract_odcs_yaml(om_contract_id)
    return dcg.import_odcs(odcs, target_contract_id=target_contract_id,
                           name_override=name_override)


_X_KEY = re.compile(r"^(x-[A-Za-z0-9-]+):", re.M)
_CP_KEY = re.compile(r"property:\s*['\"]?(x-[A-Za-z0-9-]+)")


def extension_keys(odcs_yaml: str) -> dict:
    """Return {'top_level': set(x- keys), 'custom_properties': set(x- property names)}."""
    return {
        "top_level": set(_X_KEY.findall(odcs_yaml)),
        "custom_properties": set(_CP_KEY.findall(odcs_yaml)),
    }


def diff_extensions(before: str, after: str) -> dict:
    a, b = extension_keys(before), extension_keys(after)
    return {k: {"lost": sorted(a[k] - b[k]), "kept": sorted(a[k] & b[k])} for k in a}



def dcg_odcs_for_om(odcs_yaml: str, drop_property_custom_properties: bool = True) -> str:
    """Make a DCG ODCS export acceptable to OpenMetadata 2.0.x (needs PyYAML).

    Since feat/odcs-strict DCG exports validate against the official ODCS
    v3.1.0 JSON Schema (object `description`, extensions in the root
    `customProperties[]`, schema-valid quality entries), so most of the old
    shim is gone. What is left works around OM 2.0.5 deviations from the spec:

    1. OM deserializes nested `customProperties` (on properties, quality
       entries, array items) as an object, not the spec's array, and rejects
       the document (HTTP 400). They are dropped. The constraints they carry
       (`x-cg-pattern`, `x-cg-enum`, ...) are also written as native ODCS fields,
       and OM keeps the enum (`invalidValues` quality rule) on export.
    2. OM requires root `customProperties[].value` to be a string. Object and
       list values (`x-contractgate-ontology`, `-glossary`, `-metrics`) are
       serialized to JSON text. DCG import accepts that form. OM 2.0.5 does
       not return root customProperties on export, so they don't survive OM.
    3. Legacy (pre-strict) exports still get the old fixes: a string
       `description` is wrapped as `{purpose: ...}`.
    """
    import json

    import yaml  # optional dependency, only for this shim

    doc = yaml.safe_load(odcs_yaml)
    if isinstance(doc.get("description"), str):
        doc["description"] = {"purpose": doc["description"]}

    def strip(node):
        if isinstance(node, dict):
            node.pop("customProperties", None)
            for v in node.values():
                strip(v)
        elif isinstance(node, list):
            for v in node:
                strip(v)

    if drop_property_custom_properties:
        for obj in doc.get("schema") or []:
            strip(obj)
    for cp in doc.get("customProperties") or []:
        if not isinstance(cp.get("value"), str):
            cp["value"] = json.dumps(cp.get("value"))
    return yaml.safe_dump(doc, sort_keys=False)
