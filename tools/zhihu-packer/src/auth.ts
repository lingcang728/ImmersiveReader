import { timingSafeEqual } from 'crypto';

export function hasBearerToken(header: string | undefined, expected: string): boolean {
  if (!expected || typeof header !== 'string' || !header.startsWith('Bearer ')) {
    return false;
  }
  const presented = Buffer.from(header.slice('Bearer '.length));
  const expectedBytes = Buffer.from(expected);
  // 等长比较避免把长度差异也变成时序信号；token 本身仍是 192-bit 随机值。
  return presented.length === expectedBytes.length && timingSafeEqual(presented, expectedBytes);
}
