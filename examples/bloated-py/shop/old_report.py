"""The report the nightly job printed before the dashboard existed."""


def money(value):
    cents = int(round(value * 100))
    if cents < 0:
        cents = 0
    whole, part = divmod(cents, 100)
    text = f"{whole}.{part:02d}"
    return float(text)


def render(store):
    lines = ["NIGHTLY REPORT", "=============="]
    for item in store.items:
        lines.append(f"{item['name']:<20} {money(item['price'] * item['quantity']):>8}")
    lines.append(f"{'TOTAL':<20} {money(store.total()):>8}")
    return "\n".join(lines)
