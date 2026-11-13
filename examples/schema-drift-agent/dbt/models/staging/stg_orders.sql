-- amount is USD dollars. ordered_at is unix seconds.
select
    order_id,
    customer_id,
    status,
    amount,
    currency,
    ordered_at
from {{ source('orders_feed', 'orders') }}
