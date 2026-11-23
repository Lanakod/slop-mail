import assert from "node:assert/strict";
import { test } from "node:test";
import { JSDOM } from "jsdom";
import { htmlNotice } from "../ui/html-notice.js";

test("HTML display and remote image loading need separate clicks per message", async () => {
  const dom = new JSDOM("<article><section></section></article>");
  globalThis.document = dom.window.document;
  const calls = [];
  htmlNotice(
    document.querySelector("section"),
    async (remote) => {
      calls.push(remote);
      return true;
    },
    (fn) => fn,
  );
  const button = document.querySelector("button");
  assert.deepEqual(calls, []);
  assert.equal(button.textContent, "Отобразить содержимое");
  await button.onclick();
  assert.deepEqual(calls, [false]);
  assert.equal(button.textContent, "Загрузить внешние изображения");
  await button.onclick();
  assert.deepEqual(calls, [false, true]);
  assert.ok(button.hidden);
  dom.window.close();
});
