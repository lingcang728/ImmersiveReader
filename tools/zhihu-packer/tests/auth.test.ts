import assert from "node:assert/strict";
import test from "node:test";

import { hasBearerToken } from "../src/auth.ts";

test("accepts only the exact Bearer token", () => {
  assert.equal(hasBearerToken("Bearer secret-token", "secret-token"), true);
  assert.equal(hasBearerToken("bearer secret-token", "secret-token"), false);
  assert.equal(hasBearerToken("Bearer secret-token-extra", "secret-token"), false);
  assert.equal(hasBearerToken(undefined, "secret-token"), false);
});
