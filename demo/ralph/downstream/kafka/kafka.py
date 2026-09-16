import os

from kafi.streams.streams import Streams
from kafi.kafka.cluster.cluster import Cluster
from kafi.fs.local.local import Local

orders_topic_str = "orders"
customers_topic_str = "customers"
products_topic_str = "products"

# Clean topics only — ContractGate bridge writes here from *.raw (RFC-092).
_BOOTSTRAP = os.environ.get("KAFKA_BOOTSTRAP", "127.0.0.1:9092")

def streams(sink_fun, emulated=False):
    if emulated:
        # "Connect" to emulated Kafka on local disk
        c = Local({"kafka": {"root.dir": "/tmp"}})
    else:
        # Connect to a local Kafka cluster (Redpanda in demo/ralph compose)
        c = Cluster({"kafka": {"bootstrap.servers": _BOOTSTRAP}, "kafi": {"consume_batch_size": 100}})

    # Read orders from orders topic
    orders_tn = (
        Streams.source(c, orders_topic_str).to_zSet(Streams.from_debezium)
        .map(lambda r: {
            "id": r["value"]["id"],
            "product_id": r["value"]["product_id"],
            "customer_id": r["value"]["customer_id"],
            "status": r["value"]["status"],
            "ts": r["value"]["ts"]
        })
        .distinct()
    )

    # Consume customers.
    customers_tn = (
        Streams.source(c, customers_topic_str).to_zSet(Streams.from_debezium)
        .map(lambda r: {
            "id": r["value"]["id"],
            "first_name": r["value"]["first_name"],
            "last_name": r["value"]["last_name"],
            "street_address": r["value"]["street_address"],
            "state": r["value"]["state"],
            "zip_code": r["value"]["zip_code"]
            })
        .distinct()
    )

    # Consume products.
    products_tn = (
        Streams.source(c, products_topic_str).to_zSet(Streams.from_debezium)
        .map(lambda r: {
            "id": r["value"]["id"],
            "brand": r["value"]["brand"],
            "name": r["value"]["name"],
            "sale_price": r["value"]["sale_price"]
            })
        .distinct()
    )

    # Join the three topics
    join_tn = (
        orders_tn
        .join(
            customers_tn,
            lambda l_r: l_r["customer_id"],
            lambda r_r: r_r["id"],
            lambda l_r, r_r: {**l_r, "customer": r_r}
        )
        .join(
            products_tn,
            lambda l_r: l_r["product_id"],
            lambda r_r: r_r["id"],
            lambda l_r, r_r: {"value": {**l_r,
                                        "product": r_r}}
        )
    )

    # Create sink
    sink_tn = join_tn.sink_fun(sink_fun, "sink")

    # Build the topology
    tn = Streams.build(sink_tn).from_zSet(Streams._to_records)

    # Start the Kafi Streams thread
    stop_fun = Streams.start_streams(tn)

    return stop_fun
