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

    Two incompatibilities found against OM 2.0.5:
    1. DCG exports the top-level `description` as a string; OM requires the ODCS
       object form (`purpose`/`usage`/`limitations`). Wrapped as `{purpose: ...}`.
    2. OM rejects property-level `customProperties` arrays (HTTP 400), where DCG
       keeps `x-cg-pattern`/`x-cg-min`/`x-cg-enum`. OM does not round-trip
       customProperties anyway, so they are dropped. The same constraints still
       travel in the top-level `x-contractgate-ontology` block.
    """
    import yaml  # optional dependency, only for this shim

    doc = yaml.safe_load(odcs_yaml)
    if isinstance(doc.get("description"), str):
        doc["description"] = {"purpose": doc["description"]}
    if drop_property_custom_properties:
        for obj in doc.get("schema") or []:
            for prop in obj.get("properties") or []:
                prop.pop("customProperties", None)
    return yaml.safe_dump(doc, sort_keys=False)
