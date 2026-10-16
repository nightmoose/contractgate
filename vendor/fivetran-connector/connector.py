"""Fivetran Connector SDK source for ContractGate passed events.

ContractGate is DataContractGate and AISchemaGate sharing one engine. This
connector reads only records that passed validation, from
``GET /v1/contracts/{id}/passed``, and upserts one table per contract.
"""

from __future__ import annotations

import copy
import json
import re
import time
from datetime import datetime, timezone
from urllib.parse import quote, urlparse

import requests
from fivetran_connector_sdk import ConfigurationForm, Connector, Test, form_field
from fivetran_connector_sdk import Logging as log
from fivetran_connector_sdk import Operations as op
from requests.exceptions import ChunkedEncodingError, ConnectionError, Timeout

PROD_BASE_URL = "https://api.datacontractgate.com"
PAGE_LIMIT = 1000
MAX_ATTEMPTS = 5
MAX_BACKOFF_SECONDS = 60.0
REQUEST_TIMEOUT = (10, 60)
PAID_PLAN_REQUIRED = "paid plan required"

_TABLE_NAME = re.compile(r"[^a-z0-9]+")
_CUSTOM_TABLE_NAME = re.compile(r"^[a-z_][a-z0-9_]*$")
_LOCAL_HTTP_HOSTS = frozenset({"localhost", "127.0.0.1"})
_CONTRACT_UUID = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
    re.IGNORECASE,
)
_RETRYABLE_REQUEST_ERRORS = (
    Timeout,
    ConnectionError,
    ChunkedEncodingError,
)


class ContractGateSyncError(RuntimeError):
    """A sync failure that Fivetran should surface to the connection."""


class PaidPlanRequiredError(ContractGateSyncError):
    """The API key's plan cannot export passed events."""

    def __init__(self, contract_id: str) -> None:
        super().__init__(f"{PAID_PLAN_REQUIRED} to export passed events for contract {contract_id}")


class UnknownContractError(ContractGateSyncError):
    """The contract id is not visible to this API key."""

    def __init__(self, contract_id: str) -> None:
        super().__init__(f"contract {contract_id} is unknown")


class CursorRejected(Exception):
    """HTTP 410, or HTTP 400 whose error says the cursor is invalid."""

    def __init__(self, status_code: int) -> None:
        super().__init__(f"cursor rejected with HTTP {status_code}")
        self.status_code = status_code


def validate_configuration(configuration: dict) -> dict:
    """Return a normalized configuration or raise ValueError.

    ``base_url`` defaults to production when omitted or blank, and must be
    ``https://`` except for ``http://localhost`` and ``http://127.0.0.1``.
    ``table_names`` is an optional ``contract_id=name`` list. The API key is
    never included in the error text.
    """
    if not isinstance(configuration, dict):
        raise ValueError("configuration must be a JSON object")

    api_key = configuration.get("api_key")
    if not isinstance(api_key, str) or not api_key.strip():
        raise ValueError("api_key is required")

    raw_ids = configuration.get("contract_ids")
    if not isinstance(raw_ids, str) or not raw_ids.strip():
        raise ValueError("contract_ids is required")

    contract_ids: list[str] = []
    for part in raw_ids.split(","):
        contract_id = part.strip()
        if not contract_id:
            raise ValueError("contract_ids contains an empty contract id")
        contract_id = _require_contract_uuid(contract_id)
        if contract_id not in contract_ids:
            contract_ids.append(contract_id)

    base_raw = configuration.get("base_url")
    if base_raw is None or (isinstance(base_raw, str) and not base_raw.strip()):
        base_url = PROD_BASE_URL
    elif not isinstance(base_raw, str):
        raise ValueError("base_url must be a string")
    else:
        base_url = _normalize_base_url(base_raw)

    table_names = _parse_table_names(configuration.get("table_names"), contract_ids)

    return {
        "api_key": api_key.strip(),
        "contract_ids": contract_ids,
        "base_url": base_url,
        "table_names": table_names,
    }


def configuration_check(configuration: dict):
    """Setup test that rejects a configuration which cannot sync."""
    try:
        validate_configuration(configuration)
    except ValueError as exc:
        return Test.failure(str(exc))
    return Test.success()


def configuration_form():
    """Dashboard fields for the API key, contract ids, base URL, and table names."""
    form = ConfigurationForm()
    form.add_field(
        form_field.TextField(
            "api_key",
            "API key",
            form_field.TextField.password,
            required=True,
            description=(
                "ContractGate API key. Sent as the x-api-key header. A paid plan is required."
            ),
        )
    )
    form.add_field(
        form_field.TextField(
            "contract_ids",
            "Contract IDs",
            required=True,
            description="Comma-separated contract UUIDs. Each one is synced to its own table.",
            placeholder="550e8400-e29b-41d4-a716-446655440000",
        )
    )
    form.add_field(
        form_field.TextField(
            "base_url",
            "API base URL",
            required=False,
            description=(
                f"Defaults to {PROD_BASE_URL}. Must be https://, except "
                "http://localhost and http://127.0.0.1 for local tests."
            ),
            placeholder=PROD_BASE_URL,
        )
    )
    form.add_field(
        form_field.TextField(
            "table_names",
            "Table names",
            required=False,
            description=(
                "Optional contract_id=name pairs, comma-separated. "
                "Omitted contracts keep passed_<id>."
            ),
            placeholder="550e8400-e29b-41d4-a716-446655440000=passed_orders",
        )
    )
    form.add_test(label="Configuration check", func=configuration_check)
    return form


def schema(configuration: dict):
    """Declare one passed-events table per configured contract."""
    config = validate_configuration(configuration)
    tables = []
    seen: dict[str, str] = {}
    for contract_id in config["contract_ids"]:
        table = resolve_table_name(contract_id, config["table_names"])
        if table in seen:
            raise ValueError(
                f"contract ids {seen[table]!r} and {contract_id!r} map to the same table {table}"
            )
        seen[table] = contract_id
        tables.append(
            {
                "table": table,
                "primary_key": ["id"],
                "columns": {
                    "id": "STRING",
                    "contract_version": "STRING",
                    "created_at": "UTC_DATETIME",
                    "payload": "JSON",
                },
            }
        )
    return tables


def update(configuration: dict, state: dict):
    """Page each contract from its saved cursor and checkpoint after every page."""
    config = validate_configuration(configuration)
    sync_state = _prepare_state(state)
    log.info(f"Syncing passed events for {len(config['contract_ids'])} contract(s)")
    session = requests.Session()
    try:
        for contract_id in config["contract_ids"]:
            _sync_contract(session, config, sync_state, contract_id)
    finally:
        session.close()


def _require_contract_uuid(contract_id: str) -> str:
    """Return the lowercase UUID, or raise ValueError naming the rejected id."""
    if _CONTRACT_UUID.fullmatch(contract_id) is None:
        raise ValueError(f"contract id {contract_id!r} is not a UUID")
    return contract_id.lower()


def table_name_for(contract_id: str) -> str:
    """Return passed_<sanitized contract id> using lowercase letters, digits, and underscores."""
    sanitized = _TABLE_NAME.sub("_", contract_id.strip().lower()).strip("_")
    if not sanitized:
        raise ValueError(f"contract id {contract_id!r} cannot be used as a table name")
    return f"passed_{sanitized}"


def resolve_table_name(contract_id: str, table_names: dict[str, str]) -> str:
    """Return a configured name, or the stable passed_<id> name when none is set."""
    override = table_names.get(contract_id)
    if override:
        return override
    return table_name_for(contract_id)


def _parse_table_names(raw: object, contract_ids: list[str]) -> dict[str, str]:
    """Parse ``contract_id=name`` pairs. Blank means every contract keeps passed_<id>."""
    if raw is None or (isinstance(raw, str) and not raw.strip()):
        mapping: dict[str, str] = {}
        _reject_duplicate_tables(contract_ids, mapping)
        return mapping
    if not isinstance(raw, str):
        raise ValueError("table_names must be a string")

    mapping = {}
    allowed = set(contract_ids)
    for part in raw.split(","):
        entry = part.strip()
        if not entry:
            raise ValueError("table_names contains an empty entry")
        contract_raw, separator, name_raw = entry.partition("=")
        if not separator or not contract_raw.strip() or not name_raw.strip():
            raise ValueError(f"table_names entry {entry!r} must be contract_id=name")
        try:
            contract_id = _require_contract_uuid(contract_raw.strip())
        except ValueError as exc:
            raise ValueError(f"table_names {exc}") from exc
        if contract_id not in allowed:
            raise ValueError(f"table_names contract id {contract_id} is not in contract_ids")
        if contract_id in mapping:
            raise ValueError(f"table_names lists contract id {contract_id} more than once")
        name = name_raw.strip()
        if _CUSTOM_TABLE_NAME.fullmatch(name) is None:
            raise ValueError(
                f"table name {name!r} must start with a letter or underscore and use only "
                "lowercase letters, numbers, and underscores"
            )
        mapping[contract_id] = name
    _reject_duplicate_tables(contract_ids, mapping)
    return mapping


def _reject_duplicate_tables(contract_ids: list[str], table_names: dict[str, str]) -> None:
    seen: dict[str, str] = {}
    for contract_id in contract_ids:
        table = resolve_table_name(contract_id, table_names)
        if table in seen:
            raise ValueError(
                f"contract ids {seen[table]!r} and {contract_id!r} map to the same table {table}"
            )
        seen[table] = contract_id


def _normalize_base_url(value: str) -> str:
    candidate = value.strip()
    parsed = urlparse(candidate)
    hostname = parsed.hostname
    if parsed.scheme not in {"https", "http"} or not parsed.netloc or not hostname:
        raise ValueError("base_url must be an absolute https URL")
    if parsed.username or parsed.password:
        raise ValueError("base_url must not include credentials")
    if parsed.query or parsed.fragment:
        raise ValueError("base_url must not include a query or fragment")
    if parsed.path not in {"", "/"}:
        raise ValueError("base_url must not include a path")
    if parsed.scheme != "https" and hostname not in _LOCAL_HTTP_HOSTS:
        raise ValueError(
            "base_url must use https:// (http:// is allowed only for localhost and 127.0.0.1)"
        )
    return f"{parsed.scheme}://{parsed.netloc}"


def _prepare_state(state: dict) -> dict:
    """Copy state and drop any cursor that is missing or null."""
    if not isinstance(state, dict):
        prepared: dict = {"cursors": {}}
        return prepared
    prepared = copy.deepcopy(state)
    raw_cursors = prepared.get("cursors")
    cleaned: dict[str, str] = {}
    if isinstance(raw_cursors, dict):
        for contract_id, cursor in raw_cursors.items():
            usable = _usable_cursor(cursor)
            if isinstance(contract_id, str) and usable:
                cleaned[contract_id] = usable
    prepared["cursors"] = cleaned
    return prepared


def _usable_cursor(value: object) -> str | None:
    if isinstance(value, str):
        stripped = value.strip()
        if stripped:
            return stripped
    return None


def _cursor_for(state: dict, contract_id: str) -> str | None:
    return _usable_cursor(state["cursors"].get(contract_id))


def _set_cursor(state: dict, contract_id: str, cursor: str) -> None:
    usable = _usable_cursor(cursor)
    if usable:
        state["cursors"][contract_id] = usable


def _drop_cursor(state: dict, contract_id: str) -> None:
    state["cursors"].pop(contract_id, None)


def _checkpoint(state: dict) -> None:
    """Persist cursors. Null and blank cursors are removed and never written."""
    cursors = state.get("cursors")
    if isinstance(cursors, dict):
        for contract_id, cursor in list(cursors.items()):
            usable = _usable_cursor(cursor)
            if usable:
                cursors[contract_id] = usable
            else:
                del cursors[contract_id]
    op.checkpoint(state=copy.deepcopy(state))


def _sleep(seconds: float) -> None:
    time.sleep(seconds)


def _backoff_seconds(attempt: int, retry_after: str | None) -> float:
    if isinstance(retry_after, str) and retry_after.strip():
        try:
            parsed = float(retry_after.strip())
        except ValueError:
            parsed = -1.0
        if parsed >= 0:
            return min(parsed, MAX_BACKOFF_SECONDS)
    return min(float(2**attempt), MAX_BACKOFF_SECONDS)


def _sync_contract(session: requests.Session, config: dict, state: dict, contract_id: str) -> None:
    restarted = False
    while True:
        cursor = _cursor_for(state, contract_id)
        try:
            _sync_pages(session, config, state, contract_id, cursor)
            return
        except CursorRejected as exc:
            if not cursor or restarted:
                when = "after a full re-sync" if restarted else "without a cursor"
                raise ContractGateSyncError(
                    f"ContractGate rejected the passed-events request for contract "
                    f"{contract_id} (HTTP {exc.status_code}) {when}"
                ) from exc
            log.warning(
                f"HTTP {exc.status_code} for contract {contract_id}; "
                "dropping the saved cursor and re-syncing from the start"
            )
            _drop_cursor(state, contract_id)
            _checkpoint(state)
            restarted = True


def _sync_pages(
    session: requests.Session,
    config: dict,
    state: dict,
    contract_id: str,
    cursor: str | None,
) -> None:
    table = resolve_table_name(contract_id, config["table_names"])
    seen: set[str] = set()
    while True:
        page = _fetch_page(session, config, contract_id, cursor)
        events = page.get("events")
        if not isinstance(events, list):
            raise ContractGateSyncError(
                f"ContractGate passed-events page for contract {contract_id} has no events list"
            )
        for event in events:
            op.upsert(table=table, data=_event_row(event))

        # next_cursor is a resume cursor: the last event on a non-empty page,
        # or the request cursor echoed on an empty page. Save every non-null
        # value, including on the last page. Never store null.
        next_cursor = _usable_cursor(page.get("next_cursor"))
        if next_cursor:
            _set_cursor(state, contract_id, next_cursor)
        _checkpoint(state)

        has_more = page.get("has_more") is True
        log.info(
            f"Synced {len(events)} passed events for contract {contract_id} (has_more={has_more})"
        )
        if not has_more:
            return
        if not next_cursor:
            raise ContractGateSyncError(
                f"ContractGate reported more passed events for contract {contract_id} "
                "without a new cursor"
            )
        if next_cursor in seen or next_cursor == cursor:
            raise ContractGateSyncError(
                f"ContractGate repeated a passed-event cursor for contract {contract_id}"
            )
        if cursor:
            seen.add(cursor)
        seen.add(next_cursor)
        cursor = next_cursor


def _fetch_page(
    session: requests.Session,
    config: dict,
    contract_id: str,
    cursor: str | None,
) -> dict:
    url = f"{config['base_url']}/v1/contracts/{quote(contract_id, safe='')}/passed"
    params: dict[str, object] = {"limit": PAGE_LIMIT}
    if cursor:
        params["cursor"] = cursor
    headers = {"x-api-key": config["api_key"], "Accept": "application/json"}

    for attempt in range(MAX_ATTEMPTS):
        try:
            response = session.get(url, params=params, headers=headers, timeout=REQUEST_TIMEOUT)
        except _RETRYABLE_REQUEST_ERRORS:
            if attempt + 1 >= MAX_ATTEMPTS:
                raise ContractGateSyncError(
                    f"ContractGate request failed for contract {contract_id}"
                ) from None
            delay = _backoff_seconds(attempt, None)
            log.warning(f"Retrying passed-events request for contract {contract_id}")
            _sleep(delay)
            continue

        status = response.status_code
        if status == 410 or (status == 400 and _error_says_invalid_cursor(response)):
            raise CursorRejected(status)
        if status == 400:
            raise ContractGateSyncError(
                "ContractGate rejected the passed-events request for contract "
                f"{contract_id}: {_error_text(response, config['api_key'])}"
            )
        if status == 403:
            raise PaidPlanRequiredError(contract_id)
        if status == 404:
            raise UnknownContractError(contract_id)
        if status == 429 or status >= 500:
            if attempt + 1 >= MAX_ATTEMPTS:
                raise ContractGateSyncError(
                    f"ContractGate returned HTTP {status} for contract {contract_id} after retries"
                )
            retry_after = response.headers.get("Retry-After") if response.headers else None
            delay = _backoff_seconds(attempt, retry_after)
            log.warning(
                f"Retrying passed-events request for contract {contract_id} after HTTP {status}"
            )
            _sleep(delay)
            continue
        if status == 401:
            raise ContractGateSyncError(
                f"ContractGate rejected the API key for contract {contract_id}"
            )
        if status != 200:
            raise ContractGateSyncError(
                f"ContractGate returned HTTP {status} for contract {contract_id}"
            )
        try:
            body = response.json()
        except ValueError as exc:
            raise ContractGateSyncError(
                f"ContractGate returned a non-JSON passed-events page for contract {contract_id}"
            ) from exc
        if not isinstance(body, dict):
            raise ContractGateSyncError(
                f"ContractGate returned an unexpected passed-events page for contract {contract_id}"
            )
        return body

    raise ContractGateSyncError(f"ContractGate request failed for contract {contract_id}")


def _response_error(response: requests.Response) -> str | None:
    try:
        body = response.json()
    except ValueError:
        return None
    if not isinstance(body, dict):
        return None
    error = body.get("error")
    if not isinstance(error, str) or not error.strip():
        return None
    return error.strip()


def _error_says_invalid_cursor(response: requests.Response) -> bool:
    error = _response_error(response)
    return error is not None and "invalid cursor" in error.lower()


def _error_text(response: requests.Response, api_key: str) -> str:
    error = _response_error(response)
    if error is None:
        return f"HTTP {response.status_code}"
    if api_key:
        error = error.replace(api_key, "[redacted]")
    return error


def _event_row(event: object) -> dict:
    if not isinstance(event, dict):
        raise ContractGateSyncError("passed event was not an object")
    event_id = event.get("id")
    if not isinstance(event_id, str) or not event_id:
        raise ContractGateSyncError("passed event is missing id")
    version = event.get("contract_version")
    if version is not None and not isinstance(version, str):
        version = str(version)
    return {
        "id": event_id,
        "contract_version": version,
        "created_at": _parse_created_at(event.get("created_at")),
        "payload": event.get("payload"),
    }


def _parse_created_at(value: object) -> datetime:
    if isinstance(value, datetime):
        parsed = value
    elif isinstance(value, str):
        text = value.strip()
        if text.endswith("Z"):
            text = f"{text[:-1]}+00:00"
        try:
            parsed = datetime.fromisoformat(text)
        except ValueError as exc:
            raise ContractGateSyncError("passed event created_at is not a UTC timestamp") from exc
    else:
        raise ContractGateSyncError("passed event created_at is not a UTC timestamp")
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed.astimezone(timezone.utc)


connector = Connector(update=update, schema=schema, configuration_form=configuration_form)

if __name__ == "__main__":
    with open("configuration.json", encoding="utf-8") as configuration_file:
        connector.debug(configuration=json.load(configuration_file))
