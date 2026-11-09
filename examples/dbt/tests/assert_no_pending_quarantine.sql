{{ config(severity='error') }}
{# Use severity='warn' above for a soft signal instead of failing the run. #}

-- Fails when DataContractGate has pending quarantine rows for the watched
-- contracts inside the lookback window. Self-host Postgres only.
--
-- Configure via dbt vars:
--   dbt test --vars '{
--     "dcg_quarantine_contract_ids": ["11111111-1111-1111-1111-111111111111"],
--     "dcg_quarantine_lookback_hours": 24,
--     "dcg_quarantine_direction": null
--   }'

with pending as (
  select
    qe.id,
    qe.contract_id,
    qe.direction,
    qe.created_at,
    qe.violation_count
  from {{ source('datacontractgate', 'quarantine_events') }} as qe
  where qe.status = 'pending'
    and qe.created_at >= (
      now() - make_interval(
        hours => {{ var('dcg_quarantine_lookback_hours', 24) }}
      )
    )
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
