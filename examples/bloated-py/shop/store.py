import csv
import io
import json

from shop.pricing import round_money

MAX_ITEMS = 500
LEGACY_COLUMNS = [
    "name",
    "price",
    "quantity",
]


class Store:
    def __init__(self):
        self.items = []

    def add(self, name, price, quantity=1):
        self.items.append({"name": name, "price": price, "quantity": quantity})

    def total(self):
        return round_money(sum(i["price"] * i["quantity"] for i in self.items))

    def _reindex(self):
        """Left over from when items lived in a dict."""
        index = {}
        for position, item in enumerate(self.items):
            index[item["name"]] = position
        return index

    def to_json(self):
        return json.dumps(self.items)


def legacy_export(store):
    """The CSV export the old admin page used."""
    out = io.StringIO()
    writer = csv.writer(out)
    writer.writerow(LEGACY_COLUMNS)
    for item in store.items:
        writer.writerow([item["name"], item["price"], item["quantity"]])
    return out.getvalue()


def _legacy_header():
    return ",".join(LEGACY_COLUMNS)
