export function clamp(value: number, low: number, high: number): number {
  if (value < low) {
    return low;
  }
  if (value > high) {
    return high;
  }
  return value;
}

export function percent(value: number, share: number): number {
  return (value * share) / 100;
}

export const average = (values: number[]): number =>
  values.reduce((sum, value) => sum + value, 0) / values.length;
