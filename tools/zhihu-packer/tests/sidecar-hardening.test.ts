import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { logger } from "../src/utils.ts";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

test("P2-27④: zhihu-packer.log rotates to .1 once it exceeds 10MB", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "zhihu-log-"));
  const logFile = path.join(dir, "zhihu-packer.log");
  const original = path.resolve(process.cwd(), "zhihu-packer.log");
  try {
    logger.setLogFile(logFile);
    fs.writeFileSync(logFile, "x".repeat(11 * 1024 * 1024));
    logger.info("after-rotation-marker");
    assert.equal(fs.existsSync(`${logFile}.1`), true, "oversized log must rotate to .1");
    assert.ok(fs.statSync(`${logFile}.1`).size > 10 * 1024 * 1024);
    const fresh = fs.readFileSync(logFile, "utf8");
    assert.ok(fresh.includes("after-rotation-marker"), "new line lands in a fresh log file");
    assert.ok(fresh.length < 1024, "rotated log file starts small");

    // 再超限时只保留一代：.1 被覆盖，不继续累积 .2/.3。
    fs.writeFileSync(logFile, "y".repeat(11 * 1024 * 1024));
    logger.info("second-rotation");
    assert.equal(fs.readdirSync(dir).filter(f => f.endsWith(".1")).length, 1);
    assert.ok(fs.readFileSync(`${logFile}.1`, "utf8").startsWith("y"), "old generation is replaced");
  } finally {
    logger.setLogFile(original);
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("P2-27⑥: scrapePeopleIndex detaches its response listener from a finally block", () => {
  const indexer = fs.readFileSync(path.join(root, "src", "indexer.ts"), "utf8");
  const onIdx = indexer.indexOf("page.on('response', handleResponse)");
  assert.ok(onIdx >= 0, "response listener is registered");
  const finallyIdx = indexer.indexOf("} finally {", onIdx);
  assert.ok(finallyIdx > onIdx, "scrape body must run inside try so finally can detach");
  assert.ok(
    indexer.slice(finallyIdx).includes("page.off('response', handleResponse)"),
    "page.off must run in finally so error paths cannot leak listeners",
  );
});

test("P2-27②: long waits in indexer and scheduler poll the task-stop signal", () => {
  const scheduler = fs.readFileSync(path.join(root, "src", "scheduler.ts"), "utf8");
  const indexer = fs.readFileSync(path.join(root, "src", "indexer.ts"), "utf8");
  // 30-120s 风控冷却 / 节流休息必须走可中断等待
  assert.ok(scheduler.includes("randomSleepUnlessStopped(taskId, 60000, 120000)"), "risk-control cooldown must be interruptible");
  assert.ok(scheduler.includes("randomSleepUnlessStopped(taskId, 30000, 60000)"), "throttle rest must be interruptible");
  assert.ok(scheduler.includes("sleepUnlessStopped(taskId, delay)"), "retry backoff must be interruptible");
  // 索引滚动把任务停止检查传给 scraper
  assert.ok(scheduler.includes("() => checkTaskStopped(taskId)"), "scrapePeopleIndex receives the stop check");
  // scraper 的滚动等待按轮询片切片
  assert.ok(indexer.includes("shouldStop?.()"), "index scroll loop polls the stop signal");
});

test("P2-27⑤: /health probes the database instead of unconditionally reporting ok", () => {
  const server = fs.readFileSync(path.join(root, "src", "server.ts"), "utf8");
  const health = server.slice(server.indexOf("app.get('/health'"), server.indexOf("app.get('/api/status'"));
  assert.ok(health.includes("ensureDbHealthy"), "health endpoint must probe the database");
  assert.ok(health.includes("503"), "unhealthy database must produce 503");
});
