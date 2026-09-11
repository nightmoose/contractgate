import copy, random

from datagen.constants.product_ids import product_id_str_list
from datagen.constants.customer_ids import customer_id_str_list
from datagen.constants.ips import ip_str_list

# from constants.product_ids import product_id_str_list
# from constants.customer_ids import customer_id_str_list
# from constants.ips import ip_str_list

#

class ShoeClickstreamGenerator:
    def __init__(self):
        self.ts_int = 1788854186242  # Sep 8, 2026
        self.ts_step_int = 100

    def generate(self, n_int=1):
        m_list = []
        #
        while len(m_list) < n_int:
            k_r = None
            #
            click_dict = {
                "product_id": random.choice(product_id_str_list),
                "user_id": random.choice(customer_id_str_list),
                "view_time": random.randint(10, 120),
                "ip": random.choice(ip_str_list),
                "ts": self.ts_int
            }
            #
            v_r = {"before": None, "after": copy.deepcopy(click_dict), "op": "c"}
            #
            self.ts_int += self.ts_step_int
            #
            m = {"key": k_r, "value": v_r}
            #
            m_list.append(m)
        #
        return m_list

if __name__ == "__main__":
    generator = ShoeClickstreamGenerator()
    #
    m_list = generator.generate(n_int=20)
    #
    for m in m_list:
        print(m)
