import assert from "node:assert/strict";
import { test } from "node:test";
import { label, total } from "../app.js";

test("total", () => {
  assert.equal(total([{ price: 350, quantity: 2 }, { price: 800, quantity: 1 }]), 1500);
});

test("label", () => {
  assert.equal(label(1500), "15.00");
});
