export function describe(title: string): string {
  return title.trim().toUpperCase();
}

export function limit(value: number, low: number, high: number): number {
  if (value < low) {
    return low;
  }
  if (value > high) {
    return high;
  }
  return value;
}
