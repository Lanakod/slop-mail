import { test } from "node:test";
import assert from "node:assert/strict";
import { updateChecks } from "../ui/updates.js";

test("scheduled and manual checks share a request; disabling cancels further alerts", async () => {
  let hours = 1,
    callback,
    delay,
    calls = 0,
    finish;
  const alerts = [],
    data = new Map();
  const checks = updateChecks({
    hours: () => hours,
    fetchVersion: () => {
      calls++;
      return new Promise((resolve) => {
        finish = resolve;
      });
    },
    announce: (version) => alerts.push(version),
    storage: {
      getItem: (key) => data.get(key),
      setItem: (key, value) => data.set(key, value),
    },
    clock: {
      clearTimeout: () => {
        callback = null;
      },
      setTimeout: (fn, ms) => {
        callback = fn;
        delay = ms;
        return 1;
      },
    },
  });
  checks.schedule();
  assert.equal(delay, 0);
  const background = callback();
  const manual = checks.check();
  await Promise.resolve();
  assert.equal(calls, 1);
  finish("0.5.2");
  assert.equal(await manual, "0.5.2");
  await background;
  assert.deepEqual(alerts, ["0.5.2"]);
  assert.ok(delay > 3500000 && delay <= 3600000);
  hours = 0;
  checks.schedule();
  assert.equal(callback, null);
});
