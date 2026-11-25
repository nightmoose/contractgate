with violations as (
  select "order_id"::text as "order_id", 'required:order_id' as rule, 'order_id is required but missing' as reason
  from orders
  where "order_id" is null
  union all
  select "order_id"::text as "order_id", 'pattern:order_id' as rule, 'order_id does not match pattern ^ord_[A-Za-z0-9]{10,32}$' as reason
  from orders
  where ("order_id" is not null and not (regexp_matches("order_id", '^ord_[A-Za-z0-9]{10,32}$')))
  union all
  select "order_id"::text as "order_id", 'required:customer_id' as rule, 'customer_id is required but missing' as reason
  from orders
  where "customer_id" is null
  union all
  select "order_id"::text as "order_id", 'pattern:customer_id' as rule, 'customer_id does not match pattern ^cus_[A-Za-z0-9]{8,32}$' as reason
  from orders
  where ("customer_id" is not null and not (regexp_matches("customer_id", '^cus_[A-Za-z0-9]{8,32}$')))
  union all
  select "order_id"::text as "order_id", 'required:amount_usd' as rule, 'amount_usd is required but missing' as reason
  from orders
  where "amount_usd" is null
  union all
  select "order_id"::text as "order_id", 'range:amount_usd' as rule, 'amount_usd must be at least 0' as reason
  from orders
  where ("amount_usd" is not null and ("amount_usd" < 0))
  union all
  select "order_id"::text as "order_id", 'required:status' as rule, 'status is required but missing' as reason
  from orders
  where "status" is null
  union all
  select "order_id"::text as "order_id", 'enum:status' as rule, 'status is not one of the allowed values' as reason
  from orders
  where ("status" is not null and "status" not in ('pending', 'paid', 'shipped', 'delivered', 'cancelled'))
  union all
  select "order_id"::text as "order_id", 'required:created_at' as rule, 'created_at is required but missing' as reason
  from orders
  where "created_at" is null
  union all
  select "order_id"::text as "order_id", 'required:updated_at' as rule, 'updated_at is required but missing' as reason
  from orders
  where "updated_at" is null
  union all
  select "order_id"::text as "order_id", 'length:discount_code' as rule, 'discount_code length must be between 4 and 20' as reason
  from orders
  where ("discount_code" is not null and (length("discount_code") < 4 or length("discount_code") > 20))
  union all
  select "order_id"::text as "order_id", 'unique:order_id' as rule, 'order_id is duplicated across rows' as reason
  from orders
  where ("order_id" is not null and "order_id" in (select "order_id" from orders group by "order_id" having count(*) > 1))
  union all
  select "order_id"::text as "order_id", 'freshness:created_at' as rule, 'created_at is older than the freshness window (604800 seconds)' as reason
  from orders
  where ("created_at" is not null and "created_at" < (now() - interval '604800 seconds'))
  union all
  select "order_id"::text as "order_id", 'cross_field:updated_at' as rule, 'cross-field rule violated: updated_at >= created_at' as reason
  from orders
  where ("updated_at" is not null and "created_at" is not null and not ("updated_at" >= "created_at"))
  union all
  select "order_id"::text as "order_id", 'cross_field:discount_code' as rule, 'cross-field rule violated: discount_code required if status == "cancelled"' as reason
  from orders
  where ("status" = 'cancelled' and "discount_code" is null)
)
select *
from violations
order by "order_id", rule
