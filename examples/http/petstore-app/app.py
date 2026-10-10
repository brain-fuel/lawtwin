"""A tiny Petstore, written by hand: the real service in the HTTP twin's
record, replay and conformance tests, and an application for the OpenAPI
client twin to call.

It serves the Swagger Petstore's pets, orders and users over HTTP/1.1
with only Python's standard library, keeping them in memory as the
document describes. It knows nothing of LawTwin or LawSpec. `--bug`
breaks it on purpose, so a test can show that `lawtwin peer http call`
notices:

  forget-delete  a delete answers 200 and keeps the record
  lose-field     a read leaves out the record's status
  crash-invalid  a body that is not JSON fails with 500

    python3 app.py [--port 8000] [--bug <name>]
"""

import argparse
import json
import re
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

DATE_TIME = re.compile(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+\-]\d{2}:\d{2})")
PET_STATUS = ("available", "pending", "sold")
ORDER_STATUS = ("placed", "approved", "delivered")


class Invalid(Exception):
    """A request that breaks the document: its status and why."""

    def __init__(self, status, message):
        super().__init__(message)
        self.status = status
        self.message = message


class Store:
    """The pets, orders and users the service holds."""

    def __init__(self):
        self.lock = threading.Lock()
        self.pets, self.orders, self.users = {}, {}, {}
        self.next_pet, self.next_order = 1, 1


def integer(value, where):
    if isinstance(value, bool) or not isinstance(value, int):
        raise Invalid(400, f"{where} is not an integer")
    return value


def text(value, where):
    if not isinstance(value, str):
        raise Invalid(400, f"{where} is not a string")
    return value


def optional(record, name, check):
    if name in record and record[name] is not None:
        check(record[name], name)


def check_pet(pet):
    """A pet as the document describes it."""
    if not isinstance(pet, dict):
        raise Invalid(400, "a pet is an object")
    if "name" not in pet:
        raise Invalid(400, "name is required")
    text(pet["name"], "name")
    if not isinstance(pet.get("photoUrls"), list):
        raise Invalid(400, "photoUrls is required, an array")
    for url in pet["photoUrls"]:
        text(url, "photoUrls")
    optional(pet, "id", integer)
    optional(pet, "status", lambda v, w: v in PET_STATUS or (_ for _ in ()).throw(Invalid(400, f"{w} is not a pet status")))
    for part in ("category",):
        if pet.get(part) is not None:
            if not isinstance(pet[part], dict):
                raise Invalid(400, f"{part} is an object")
            optional(pet[part], "id", integer)
            optional(pet[part], "name", text)
    if pet.get("tags") is not None:
        if not isinstance(pet["tags"], list):
            raise Invalid(400, "tags is an array")
        for tag in pet["tags"]:
            if not isinstance(tag, dict):
                raise Invalid(400, "a tag is an object")
            optional(tag, "id", integer)
            optional(tag, "name", text)
    return pet


def check_order(order):
    if not isinstance(order, dict):
        raise Invalid(422, "an order is an object")
    for name in ("id", "petId", "quantity"):
        optional(order, name, integer)
    if order.get("quantity") is not None and not -(2**31) <= order["quantity"] < 2**31:
        raise Invalid(422, "quantity is a 32-bit integer")
    if order.get("shipDate") is not None:
        if not isinstance(order["shipDate"], str) or not DATE_TIME.fullmatch(order["shipDate"]):
            raise Invalid(422, "shipDate is not a date-time")
    if order.get("status") is not None and order["status"] not in ORDER_STATUS:
        raise Invalid(422, "status is not an order status")
    if order.get("complete") is not None and not isinstance(order["complete"], bool):
        raise Invalid(422, "complete is a boolean")
    return order


def check_user(user):
    if not isinstance(user, dict):
        raise Invalid(400, "a user is an object")
    for name in ("username", "firstName", "lastName", "email", "password", "phone"):
        optional(user, name, text)
    optional(user, "id", integer)
    optional(user, "userStatus", integer)
    return user


def clean(record):
    """A record as it is kept: without absent or null properties."""
    return {k: v for k, v in record.items() if v is not None}


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    store = Store()
    bug = None

    def log_message(self, *args):
        pass

    def send(self, status, body=None):
        data = b"" if body is None else json.dumps(body).encode()
        self.send_response(status)
        if body is not None:
            self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def body(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        if not raw.strip():
            return None
        try:
            return json.loads(raw)
        except ValueError:
            if self.bug == "crash-invalid":
                raise RuntimeError("the body is not JSON")
            raise Invalid(400, "the body is not JSON")

    def route(self, method):
        path = self.path.split("?", 1)[0].rstrip("/")
        parts = path.split("/")[1:]
        s = self.store
        with s.lock:
            if parts == ["pet"] and method == "POST":
                pet = clean(check_pet(self.body()))
                key = pet.get("id", s.next_pet)
                if key in s.pets:
                    raise Invalid(422, "a pet with this id exists")
                pet["id"] = key
                s.pets[key] = pet
                s.next_pet = max(s.next_pet, key + 1)
                return 200, pet
            if parts == ["pet"] and method == "PUT":
                pet = clean(check_pet(self.body()))
                if "id" not in pet:
                    raise Invalid(422, "id is required")
                if pet["id"] not in s.pets:
                    return 404, {"message": "no such pet"}
                s.pets[pet["id"]] = pet
                return 200, pet
            if len(parts) == 2 and parts[0] == "pet" and method in ("GET", "DELETE"):
                try:
                    key = int(parts[1])
                except ValueError:
                    raise Invalid(400, "petId is not an integer")
                if key not in s.pets:
                    return 404, {"message": "no such pet"}
                if method == "DELETE":
                    if self.bug != "forget-delete":
                        del s.pets[key]
                    return 200, None
                pet = dict(s.pets[key])
                if self.bug == "lose-field":
                    pet.pop("status", None)
                return 200, pet
            if parts == ["store", "order"] and method == "POST":
                order = self.body()
                if order is None:
                    raise Invalid(422, "an order is required")
                order = clean(check_order(order))
                key = order.get("id", s.next_order)
                if key in s.orders:
                    raise Invalid(422, "an order with this id exists")
                order["id"] = key
                s.orders[key] = order
                s.next_order = max(s.next_order, key + 1)
                return 200, order
            if len(parts) == 3 and parts[:2] == ["store", "order"] and method in ("GET", "DELETE"):
                try:
                    key = int(parts[2])
                except ValueError:
                    raise Invalid(400, "orderId is not an integer")
                if key not in s.orders:
                    return 404, {"message": "no such order"}
                if method == "DELETE":
                    if self.bug != "forget-delete":
                        del s.orders[key]
                    return 200, None
                return 200, s.orders[key]
            if parts == ["user"] and method == "POST":
                user = self.body()
                if user is None:
                    raise Invalid(400, "a user is required")
                user = clean(check_user(user))
                if "username" not in user:
                    raise Invalid(400, "username is required")
                s.users[user["username"]] = user
                return 200, user
            if len(parts) == 2 and parts[0] == "user" and parts[1] not in ("login", "logout", "createWithList"):
                name = parts[1]
                if method == "GET":
                    if name not in s.users:
                        return 404, {"message": "no such user"}
                    return 200, s.users[name]
                if method == "PUT":
                    user = self.body()
                    if user is None:
                        raise Invalid(400, "a user is required")
                    user = clean(check_user(user))
                    if name not in s.users:
                        return 404, {"message": "no such user"}
                    user["username"] = name
                    s.users[name] = user
                    return 200, None
                if method == "DELETE":
                    if name not in s.users:
                        return 404, {"message": "no such user"}
                    if self.bug != "forget-delete":
                        del s.users[name]
                    return 200, None
        return 501, {"message": f"{method} {path} is not served by this Petstore"}

    def answer(self, method):
        try:
            status, body = self.route(method)
        except Invalid as e:
            status, body = e.status, {"message": e.message}
        except Exception as e:  # a defect, as a real service has them
            status, body = 500, {"message": str(e)}
        self.send(status, body)

    def do_GET(self):
        self.answer("GET")

    def do_POST(self):
        self.answer("POST")

    def do_PUT(self):
        self.answer("PUT")

    def do_DELETE(self):
        self.answer("DELETE")


def main():
    parser = argparse.ArgumentParser(description="A tiny Petstore.")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8000)
    parser.add_argument("--bug", choices=("forget-delete", "lose-field", "crash-invalid"))
    args = parser.parse_args()
    Handler.bug = args.bug
    server = ThreadingHTTPServer((args.host, args.port), Handler)
    print(f"petstore-app on http://{args.host}:{args.port}" + (f" with the bug {args.bug}" if args.bug else ""), flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
