TAX = 0.22


def round_money(value):
    cents = int(round(value * 100))
    if cents < 0:
        cents = 0
    whole, part = divmod(cents, 100)
    text = f"{whole}.{part:02d}"
    return float(text)


def with_tax(value):
    return round_money(value * (1 + TAX))


def apply_discount(value, percent):
    if percent <= 0:
        return value
    if percent >= 100:
        return 0.0
    return round_money(value * (100 - percent) / 100)
