import assert from "node:assert/strict";
import test from "node:test";

import {
  READY_PROTOCOL_VERSION,
  formatReadyLine,
  resolveSidecarPort,
  readyPayload,
} from "../src/sidecar-protocol.ts";

test("builds a versioned READY payload with a dynamic port", () => {
  assert.deepEqual(readyPayload("zhihu", 4242, 43210), {
    engine: "zhihu",
    protocolVersion: READY_PROTOCOL_VERSION,
    pid: 4242,
    port: 43210,
  });
  assert.equal(
    formatReadyLine("zhihu", 4242, 43210),
    '{"engine":"zhihu","protocolVersion":1,"pid":4242,"port":43210}\n',
  );
});

test("accepts port zero for OS-assigned binding and rejects invalid values", () => {
  assert.equal(resolveSidecarPort("0"), 0);
  assert.equal(resolveSidecarPort("43210"), 43210);
  assert.throws(() => resolveSidecarPort("-1"), /port/i);
  assert.throws(() => resolveSidecarPort("65536"), /port/i);
  assert.throws(() => resolveSidecarPort("not-a-port"), /port/i);
});
