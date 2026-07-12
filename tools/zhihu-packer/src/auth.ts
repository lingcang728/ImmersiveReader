export function hasBearerToken(header: string | undefined, expected: string): boolean {
  return Boolean(expected) && header === `Bearer ${expected}`;
}
