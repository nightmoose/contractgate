select
    order_id,
    customer_id,
    status,
    amount,
    currency,
    ordered_at
from {{ ref('stg_orders') }}
