import { test } from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { JSDOM } from "jsdom";
test("composer preserves typed text, recipient visibility and sender signature across reopening", async (t) => {
  const html = await readFile(
    new URL("../ui/index.html", import.meta.url),
    "utf8",
  );
  const dom = new JSDOM(html, { url: "http://app.local" }),
    w = dom.window;
  t.after(() => dom.window.close());
  w.HTMLDialogElement.prototype.showModal = function () {
    this.open = true;
  };
  w.HTMLDialogElement.prototype.close = function () {
    this.open = false;
  };
  Object.assign(globalThis, {
    window: w,
    document: w.document,
    localStorage: w.localStorage,
  });
  w.__composeContext = {
    profile: "local",
    slot: "new:sample",
    to: "",
    subject: "",
    body: "",
  };
  let drop;
  let draft = null,
    closeWindow,
    savedOnClose;
  const boxes = ["work", "home"].map((id) => ({
    id,
    name: id,
    address: id + "@example.org",
    signature: "signature-" + id,
  }));
  w.__TAURI__ = {
    event: { listen: async () => {}, emitTo: async () => {} },
    webview: {
      getCurrentWebview: () => ({
        onDragDropEvent: async (fn) => {
          drop = fn;
        },
      }),
    },
    window: {
      getCurrentWindow: () => ({
        onCloseRequested: async (handler) => {
          closeWindow = handler;
        },
      }),
    },
    core: {
      invoke: async (command, { op, args }) => {
        if (command === "finish_close") {
          savedOnClose = draft?.body;
          return;
        }
        switch (op) {
          case "profiles":
            return [{ id: "local", name: "Local" }];
          case "accounts":
            return boxes;
          case "preview":
            return args.to.split(",").filter(Boolean);
          case "recipient_plan":
            return ["to", "cc", "bcc"].flatMap((field) =>
              (args[field] || "")
                .split(",")
                .filter(Boolean)
                .map((email) => ({ email, field })),
            );
          case "drafts":
            return draft
              ? [
                  {
                    slot: draft.slot,
                    data: JSON.stringify(draft),
                    modified: 42,
                  },
                ]
              : [];
          case "draft":
            return draft?.slot === args.slot ? draft : null;
          case "save_draft":
            draft = args.data;
            return;
          default:
            return [];
        }
      },
    },
  };
  await import("../ui/app.js");
  const tick = () => new Promise((resolve) => setTimeout(resolve, 5));
  await tick();

  const q = (field) => w.document.querySelector('[data-field="' + field + '"]');
  await tick();
  q("body").textContent = "Typed text";
  q("body").dispatchEvent(new w.Event("input", { bubbles: true }));
  q("sender").value = "home";
  q("sender").dispatchEvent(new w.Event("change", { bubbles: true }));
  q("mode").value = "personal";
  q("to").value = "one@example.org,two@example.org";
  q("to").dispatchEvent(new w.Event("input", { bubbles: true }));
  await tick();
  assert.equal(q("body").textContent, "Typed text");
  assert.equal(q("signature").textContent, "signature-home");
  assert.match(q("preview").textContent, /2.*только свой адрес/);
  assert.equal(draft.subject, "");
  assert.equal(draft.account, "home");
  assert.equal(draft.personal, true);
  assert.equal(q("send").disabled, false);
  q("body").textContent = " \n\u200b";
  q("body").dispatchEvent(new w.Event("input", { bubbles: true }));
  await tick();
  assert.equal(q("send").disabled, true);
  drop({
    payload: { type: "drop", paths: ["/tmp/test.txt", "/tmp/test.txt"] },
  });
  await tick();
  assert.equal(q("send").disabled, false);
  assert.deepEqual(draft.files, ["/tmp/test.txt"]);
  q("body").textContent = "Saved before hiding";
  q("body").dispatchEvent(new w.Event("input", { bubbles: true }));
  let prevented = false;
  await closeWindow({
    preventDefault() {
      prevented = true;
    },
  });
  assert.equal(prevented, true);
  assert.equal(savedOnClose, "Saved before hiding");
});
