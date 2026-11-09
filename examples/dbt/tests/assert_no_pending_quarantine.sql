{{ config(severity='error') }}
{# Use severity='warn' above for a soft signal instead of failing the run. #}

-- Fails when DataContractGate has pending quarantine rows for this run.
-- Self-host Postgres only.
--
-- Send the same id on ingest and egress as X-Batch-Id (query batch_id / run_id
-- are aliases). This test defaults to dbt's invocation_id. Override it with
-- dcg_quarantine_batch_id when a different job stamped the rows.
--
-- Time-window fallback, for rows that have no batch_id:
--   dbt test --vars '{
--     "dcg_quarantine_use_time_window": true,
--     "dcg_quarantine_contract_ids": ["11111111-1111-1111-1111-111111111111"],
--     "dcg_quarantine_lookback_hours": 24,
--     "dcg_quarantine_direction": null
--   }'

{% set use_window = var('dcg_quarantine_use_time_window', false) %}
{% set batch_id = var('dcg_quarantine_batch_id', invocation_id) | string | replace("'", "''") %}

with pending as (
  select
    qe.id,
    qe.contract_id,
    qe.direction,
    qe.batch_id,
    qe.created_at,
    qe.violation_count
  from {{ source('datacontractgate', 'quarantine_events') }} as qe
  where qe.status = 'pending'
    {% if use_window %}
    and qe.created_at >= (
      now() - make_interval(
        hours => {{ var('dcg_quarantine_lookback_hours', 24) }}
      )
    )
    {% else %}
    and qe.batch_id = '{{ batch_id }}'
    {% endif %}
    {% if var('dcg_quarantine_contract_ids', none) %}
    and qe.contract_id in (
      {% for cid in var('dcg_quarantine_contract_ids') %}
        '{{ cid }}'::uuid{% if not loop.last %},{% endif %}
      {% endfor %}
    )
    {% endif %}
    {% if var('dcg_quarantine_direction', none) %}
    and qe.direction = '{{ var("dcg_quarantine_direction") }}'
    {% endif %}
)

select *
from pending
