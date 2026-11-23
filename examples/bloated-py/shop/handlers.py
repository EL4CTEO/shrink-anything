"""Called by name from app.py: getattr(handlers, "on_" + event)."""


def on_create(store):
    return len(store.items)


def on_delete(store):
    return 0
