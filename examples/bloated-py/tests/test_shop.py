import unittest

from shop.pricing import round_money, with_tax
from shop.store import Store


class ShopTest(unittest.TestCase):
    def test_total(self):
        store = Store()
        store.add("tea", 3.50, 2)
        store.add("mug", 8.00)
        self.assertEqual(store.total(), 15.0)

    def test_tax(self):
        self.assertEqual(with_tax(100), 122.0)

    def test_rounding(self):
        self.assertEqual(round_money(1.005), 1.0)
        self.assertEqual(round_money(-3), 0.0)

    def test_json(self):
        store = Store()
        store.add("tea", 3.5)
        self.assertIn("tea", store.to_json())


if __name__ == "__main__":
    unittest.main()
