import copy, random

from datagen.constants.customers import customer_dict_list

# from constants.customers import customer_dict_list

#

class ShoeCustomerGenerator:
    def __init__(self):
        self.customer_dict_list = []

    def generate(self, n_int=1):
        m_list = []
        #
        while len(m_list) < n_int:
            rand_int = random.randrange(10)
            #
            if rand_int <= 6 or self.customer_dict_list == []: # insert
                customer_dict = random.choice(customer_dict_list)
                #
                customer_dict = copy.deepcopy(customer_dict)
                self.customer_dict_list.append(customer_dict)
                #
                k_r = customer_dict["id"]
                #
                v_r = {"before": None, "after": customer_dict, "op": "c"}
                #
                m = {"key": k_r, "value": v_r}
                #
                m_list.append(m)
            elif rand_int > 6 and rand_int <= 8: # update
                customer_index_int = random.randint(0, len(self.customer_dict_list) - 1)
                #
                old_customer_dict = self.customer_dict_list.pop(customer_index_int)
                #
                customer_dict = copy.deepcopy(old_customer_dict)
                
                update_choice = random.choice(["address", "phone", "email"])
                if update_choice == "address":
                    customer_dict["street_address"] = f"{random.randint(1, 999)} Main Street"
                    customer_dict["zip_code"] = str(random.randint(10000, 99999))
                elif update_choice == "phone":
                    customer_dict["phone"] = f"{random.randint(100, 999)}-{random.randint(100, 999)}-{random.randint(1000, 9999)}"
                else:
                    customer_dict["email"] = f"updated_{customer_dict['email']}"
                #
                self.customer_dict_list.append(customer_dict)
                #
                k_r = customer_dict["id"]
                #
                v_r = {"before": old_customer_dict, "after": customer_dict, "op": "u"}
                #
                m = {"key": k_r, "value": v_r}
                #
                m_list.append(m)
            else: # delete
                customer_index_int = random.randint(0, len(self.customer_dict_list) - 1)
                #
                customer_dict = self.customer_dict_list.pop(customer_index_int)
                k_r = customer_dict["id"]
                #
                v_r = {"before": customer_dict, "after": None, "op": "d"}
                #
                m = {"key": k_r, "value": v_r}
                #
                m_list.append(m)
        #
        return m_list

if __name__ == "__main__":
    generator = ShoeCustomerGenerator()
    #
    m_list = generator.generate(n_int=20)
    #
    for m in m_list:
        print(m)