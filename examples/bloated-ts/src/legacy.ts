// The v1 checkout, kept "just in case" since the rewrite.
export function checkoutV1(prices: number[]): number {
  let total = 0;
  for (const price of prices) {
    total += price;
  }
  return Math.round(total * 100) / 100;
}
