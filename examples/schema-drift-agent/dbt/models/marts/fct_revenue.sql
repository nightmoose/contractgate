select
    date_trunc('day', to_timestamp(ordered_at)) as revenue_day,
    currency,
    sum(amount) as revenue_usd,
    count(*) as order_count
from {{ ref('stg_orders') }}
where status in ('paid', 'shipped')
group by 1, 2
