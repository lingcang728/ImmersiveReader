export const READY_PROTOCOL_VERSION = 1;

export interface ReadyPayload {
  engine: string;
  protocolVersion: number;
  pid: number;
  port: number;
}

export function resolveSidecarPort(value = process.env.IMMERSIVE_SIDECAR_PORT ?? "0"): number {
  if (!/^\d+$/.test(value.trim())) {
    throw new Error(`Invalid sidecar port: ${value}`);
  }
  const port = Number(value);
  if (!Number.isInteger(port) || port < 0 || port > 65535) {
    throw new Error(`Invalid sidecar port: ${value}`);
  }
  return port;
}

export function readyPayload(engine: string, pid: number, port: number): ReadyPayload {
  if (!engine || !Number.isInteger(pid) || pid <= 0 || !Number.isInteger(port) || port <= 0 || port > 65535) {
    throw new Error("READY payload requires a positive PID and bound port");
  }
  return {
    engine,
    protocolVersion: READY_PROTOCOL_VERSION,
    pid,
    port,
  };
}

export function formatReadyLine(engine: string, pid: number, port: number): string {
  return `${JSON.stringify(readyPayload(engine, pid, port))}\n`;
}

export function writeReady(engine: string, pid: number, port: number): void {
  process.stdout.write(formatReadyLine(engine, pid, port));
}
