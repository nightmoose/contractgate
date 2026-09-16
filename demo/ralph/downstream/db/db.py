from functools import cached_property
import time

import lancedb
from lancedb.embeddings import get_registry, register, TextEmbeddingFunction
from lancedb.pydantic import LanceModel, Vector

from fastembed import TextEmbedding

table_str = "customer-context"
embedding_str = "snowflake/snowflake-arctic-embed-xs"

# FastEmbed/LanceDB integration
@register("fastembed")
class FastEmbedEmbeddings(TextEmbeddingFunction):
    name_str: str = embedding_str
    max_length_int: int = 512

    def generate_embeddings(self, text_str_list):
        return [numpyArrayIterable.tolist() for numpyArrayIterable in self._model.embed(text_str_list)]

    def ndims(self):
        return len(self.generate_embeddings(["test"])[0])

    @cached_property
    def _model(self):
        return TextEmbedding(model_name=self.name_str, max_length=self.max_length_int)

embeddingFunction = get_registry().get("fastembed").create(name=embedding_str)

# Table schema
class CustomerContext(LanceModel):
    id: str
    customer_id: str
    customer_name: str
    summary: str = embeddingFunction.SourceField()
    summary_embedding: Vector(embeddingFunction.ndims()) = embeddingFunction.VectorField() # type: ignore[reportInvalidTypeForm]

#

def connect():
    # Connect to local LanceDB
    dbConnection = lancedb.connect("./lancedb_data")
    #
    return dbConnection


def create_table(dbConnection, *, overwrite: bool = False):
    # Create the table. Default: reuse existing so a warm restart keeps rows
    # for MCP demos; pass overwrite=True for a clean slate.
    mode = "overwrite" if overwrite else "create"
    try:
        table = dbConnection.create_table(table_str, schema=CustomerContext, mode=mode)
    except Exception:
        # Table already exists when mode=create
        table = dbConnection.open_table(table_str)
    return table

#

def get_sink_fun(table):
    def sink_fun(m_w_tuple_list):
        add_order_id_str_r_dict = {}
        delete_order_id_str_set = set()
        for m, w in m_w_tuple_list:
            v = m["value"]
            order_id_str = str(v["id"])
            customer_id_str = v["customer"]["id"]
            customer_name_str = f"{v["customer"]["first_name"]} {v["customer"]["last_name"]}"
            #
            if w == 1:
                summary_str = f"""
                Order #{v["id"]} for customer {v["customer"]["id"]} (product {v["product"]["id"]}) is in status {v["status"]} at timestamp {v["ts"]}. Customer name: {v["customer"]["first_name"]} {v["customer"]["last_name"]}, address: {v["customer"]["street_address"]}, {v["customer"]["state"]}, {v["customer"]["zip_code"]}. Product name: {v["product"]["name"]}, brand: {v["product"]["brand"]}, sale price: ${v["product"]["sale_price"]}
                """
                
                r = {
                    "id": order_id_str,
                    "customer_id": customer_id_str,
                    "customer_name": customer_name_str,
                    "summary": summary_str,
                    "summary_embedding": embeddingFunction.generate_embeddings([summary_str])[0]
                }
        
                add_order_id_str_r_dict[order_id_str] = r
            elif w == -1:
                delete_order_id_str_set.add(order_id_str)

        order_id_str_list = list(delete_order_id_str_set)
        if order_id_str_list:
            ids_sql_str = ", ".join(f"'{i}'" for i in order_id_str_list)
            # print()
            # print(f"Deleting: {order_id_str_list}")
            table.delete(f"id IN ({ids_sql_str})")

        r_list = list(add_order_id_str_r_dict.values())
        # print()
        # print(f"Merge inserting: {d_list}")
        if r_list:
            a = time.time()
            print(f"Merge inserting {len(r_list)} rows...")
            table.merge_insert("id") \
                .when_matched_update_all() \
                .when_not_matched_insert_all() \
                .execute(r_list)
            b = time.time()
            print(f"...done ({(b - a)}s, {(b - a)/len(r_list)} per row).")

        print([r["customer_id"] for r in r_list])
    #
    return sink_fun

#

def search(dbConnection, query, id, customer_id, customer_name, limit):
    table = dbConnection.open_table(table_str)
    #
    if query:
        query_float_list = embeddingFunction.generate_embeddings([query])[0]
    else:
        query_float_list = None
    #
    where_str_list = []
    #
    if id:
        where_str_list.append(f"id = '{id}'")
    #
    if customer_id:
        where_str_list.append(f"customer_id = '{customer_id}'")
    #
    if customer_name:
        where_str_list.append(f"customer_name LIKE '%{customer_name}%'")
    #
    if where_str_list:
        where_str = " AND ".join(where_str_list)
        #
        results = table.search(query_float_list, "summary_embedding") \
                    .where(where_str) \
                    .limit(limit) \
                    .to_list()
    else:
        results = table.search(query_float_list, "summary_embedding") \
                    .limit(limit) \
                    .to_list()
    #
    return [
        {
            "summary": r["summary"],
            "score": 1.0 if "_distance" not in r else round(1.0 - r["_distance"], 4),
        }
        for r in results
    ]
