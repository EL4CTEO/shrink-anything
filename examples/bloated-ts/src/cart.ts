import { clamp, percent } from "./math.ts";
import { describe } from "./labels.ts";

export interface Item {
  name: string;
  price: number;
  quantity: number;
}

export interface OldItem {
  title: string;
  cents: number;
}

export class Cart {
  private items: Item[] = [];

  add(item: Item): void {
    this.items.push(item);
  }

  total(): number {
    return this.items.reduce((sum, item) => sum + item.price * item.quantity, 0);
  }

  discounted(off: number): number {
    return this.total() - percent(this.total(), clamp(off, 0, 100));
  }

  private recount(): number {
    let count = 0;
    for (const item of this.items) {
      count += item.quantity;
    }
    return count;
  }
}

export function formatLegacy(items: OldItem[]): string {
  const rows: string[] = [];
  for (const item of items) {
    rows.push(describe(item.title) + ": " + (item.cents / 100).toFixed(2));
  }
  return rows.join("\n");
}

export const LEGACY_CURRENCY = "ITL";
