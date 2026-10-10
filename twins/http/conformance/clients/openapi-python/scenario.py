"""The Petstore twin through a client generated from the Petstore's own
OpenAPI document by openapi-generator (python), as an application would
use it. `make openapi-client-http` generates and installs the client; the
conformance kit runs this with the twin's URL.

    python scenario.py http://127.0.0.1:8080
"""

import sys

import petstore_client
from petstore_client.rest import ApiException


def main(url):
    """Adds, reads, changes and deletes through the generated client, and checks each answer."""
    config = petstore_client.Configuration(host=url)
    with petstore_client.ApiClient(config) as client:
        pets = petstore_client.PetApi(client)
        store = petstore_client.StoreApi(client)
        users = petstore_client.UserApi(client)

        rex = pets.add_pet(petstore_client.Pet(name="Rex", photo_urls=["https://example.com/rex.png"], status="available"))
        assert rex.id is not None and rex.name == "Rex", rex
        assert pets.get_pet_by_id(rex.id) == rex

        sold = pets.update_pet(petstore_client.Pet(id=rex.id, name="Rex", photo_urls=[], status="sold"))
        assert pets.get_pet_by_id(rex.id).status == "sold", sold

        order = store.place_order(petstore_client.Order(pet_id=rex.id, quantity=1, status="placed"))
        assert store.get_order_by_id(order.id) == order

        users.create_user(petstore_client.User(username="ada", first_name="Ada"))
        assert users.get_user_by_name("ada").first_name == "Ada"

        pets.delete_pet(rex.id)
        try:
            pets.get_pet_by_id(rex.id)
            raise AssertionError("a deleted pet was found")
        except ApiException as e:
            assert e.status == 404, e

        # An operation the contract does not model yet: the twin says so.
        try:
            pets.find_pets_by_status("sold")
            raise AssertionError("findPetsByStatus answered")
        except ApiException as e:
            assert e.status == 501, e
    print("the generated client agrees with the twin")


if __name__ == "__main__":
    main(sys.argv[1])
