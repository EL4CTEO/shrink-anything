export function total(items) {
  return items.reduce((sum, item) => sum + item.price * item.quantity, 0);
}

export function label(cents) {
  return (cents / 100).toFixed(2);
}

export function legacyBanner() {
  return "SUMMER SALE";
}

if (typeof document !== "undefined") {
  document.getElementById("total").textContent = label(total([{ price: 350, quantity: 2 }]));
}
