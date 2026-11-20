import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { threadList } from "../ui/thread-list.js";
test("expansion does not open or mark mail, context action targets the child", async () => {
  const w = new JSDOM('<div id="list"></div>').window;
  globalThis.window = w;
  globalThis.document = w.document;
  const root = w.document.querySelector("#list"),
    calls = [];
  const one = { id: 1, thread: "chain", subject: "Тема", seen: 1 },
    two = { ...one, id: 2, seen: 0, subject: "Re: Тема" };
  const view = threadList({
    api: async (op, args) => {
      calls.push([op, args]);
      return [one, two];
    },
    action: (fn) => fn,
    date: () => "",
    open: () => assert.fail("unexpected open"),
    refresh: async () => {},
  });
  view.render(
    root,
    [{ ...two, topic: "Тема", unread: 1, total: 2 }],
    { account: "box" },
    null,
    "work",
  );
  assert.equal(
    root.querySelector(".branch-toggle").getAttribute("aria-expanded"),
    "false",
  );
  assert.ok(root.querySelector(".branch-head .unread"));
  await root.querySelector(".branch-toggle").onclick();
  const rows = root.querySelectorAll(".branch-children .mail-row");
  assert.equal(rows.length, 2);
  assert.ok(!rows[0].classList.contains("unread"));
  assert.ok(rows[1].classList.contains("unread"));
  assert.deepEqual(calls, [["list", { account: "box", branch: "chain" }]]);
  rows[1].dispatchEvent(
    new w.MouseEvent("contextmenu", { bubbles: true, cancelable: true }),
  );
  await w.document.querySelector('[role="menuitem"]').onclick();
  assert.deepEqual(calls.at(-1), [
    "mark_seen",
    { thread: "chain", ids: [2], seen: true },
  ]);
  w.close();
});

test("Shift range and Ctrl selection star exactly the visible selected messages", async () => {
  const w = new JSDOM('<div id="list"></div>').window;
  globalThis.window = w;
  globalThis.document = w.document;
  const rows = [1, 2, 3, 4].map((id) => ({
    id,
    thread: String(id),
    total: 1,
    seen: 0,
    subject: "Mail",
  }));
  const root = w.document.querySelector("#list");
  let chosen;
  const view = threadList({
    api: async () => [],
    action: (fn) => fn,
    date: () => "",
    open: () => {},
    refresh: () => {},
    bulk: async (...args) => {
      chosen = args;
    },
  });
  view.render(root, rows, {}, null, "work");
  const buttons = root.querySelectorAll(".mail-row");
  await buttons[0].onclick({ ctrlKey: true });
  await buttons[2].onclick({ shiftKey: true });
  await buttons[1].onclick({ metaKey: true });
  buttons[2].dispatchEvent(
    new w.MouseEvent("contextmenu", { bubbles: true, cancelable: true }),
  );
  await [...w.document.querySelectorAll('[role="menuitem"]')]
    .find((b) => b.textContent === "Поставить звезду")
    .onclick();
  assert.deepEqual(chosen, ["star", [1, 3], "work"]);
  w.close();
});
