"""A tiny shop: the entry point."""

from shop import handlers
from shop.pricing import with_tax
from shop.store import Store


def main():
    store = Store()
    store.add("tea", 3.50, 2)
    store.add("mug", 8.00, 1)
    for event in ("create", "delete"):
        getattr(handlers, "on_" + event)(store)
    print(f"total: {with_tax(store.total()):.2f}")


if __name__ == "__main__":
    main()
