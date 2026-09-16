import copy, random

from datagen.constants.products import product_dict_list

# from constants.products import product_dict_list

#

class ShoeProductGenerator:
    def __init__(self):
        self.product_dict_list = []

    def generate(self, n_int=1):
        m_list = []
        #
        while len(m_list) < n_int:
            rand_int = random.randrange(10)
            #
            if rand_int <= 6 or self.product_dict_list == []: # insert
                product_dict = random.choice(product_dict_list)
                #
                product_dict = copy.deepcopy(product_dict)
                self.product_dict_list.append(product_dict)
                #
                k_r = product_dict["id"]
                #
                v_r = {"before": None, "after": product_dict, "op": "c"}
                #
                m = {"key": k_r, "value": v_r}
                #
                m_list.append(m)
            elif rand_int > 6 and rand_int <= 8: # update
                product_index_int = random.randint(0, len(self.product_dict_list) - 1)
                #
                old_product_dict = self.product_dict_list.pop(product_index_int)
                #
                product_dict = copy.deepcopy(old_product_dict)
                product_dict["sale_price"] = round(product_dict["sale_price"] * random.uniform(0.85, 1.15), 2)
                #
                self.product_dict_list.append(product_dict)
                #
                k_r = product_dict["id"]
                #
                v_r = {"before": old_product_dict, "after": product_dict, "op": "u"}
                #
                m = {"key": k_r, "value": v_r}
                #
                m_list.append(m)
            else: # delete
                product_index_int = random.randint(0, len(self.product_dict_list) - 1)
                #
                product_dict = self.product_dict_list.pop(product_index_int)
                k_r = product_dict["id"]
                #
                v_r = {"before": product_dict, "after": None, "op": "d"}
                #
                m = {"key": k_r, "value": v_r}
                #
                m_list.append(m)
        #
        return m_list

if __name__ == "__main__":
    generator = ShoeProductGenerator()
    #
    m_list = generator.generate(n_int=20)
    #
    for m in m_list:
        print(m)
