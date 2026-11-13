-- Fails if a cents-scale amount reaches staging.
select order_id
from {{ ref('stg_orders') }}
where amount > 300
