import assert from "node:assert/strict";
import { test } from "node:test";
import { Cart } from "../src/cart.ts";
import { clamp } from "../src/math.ts";

test("total", () => {
  const cart = new Cart();
  cart.add({ name: "tea", price: 3.5, quantity: 2 });
  cart.add({ name: "mug", price: 8, quantity: 1 });
  assert.equal(cart.total(), 15);
});

test("discount", () => {
  const cart = new Cart();
  cart.add({ name: "mug", price: 10, quantity: 1 });
  assert.equal(cart.discounted(20), 8);
  assert.equal(cart.discounted(500), 0);
});

test("clamp", () => {
  assert.equal(clamp(5, 0, 3), 3);
});
