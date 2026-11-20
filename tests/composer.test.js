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
  const realInterval = globalThis.setInterval;
  globalThis.setInterval = (...args) => realInterval(...args).unref();
  t.after(() => {
    globalThis.setInterval = realInterval;
  });
  let draft = null;
  const boxes = ["work", "home"].map((id) => ({
    id,
    name: id,
    address: id + "@example.org",
    signature: "signature-" + id,
  }));
  w.__TAURI__ = {
    window: { getCurrentWindow: () => ({ onCloseRequested: async () => {} }) },
    core: {
      invoke: async (_, { op, args }) => {
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
  const click = (id) => w.document.getElementById(id).click();
  const q = (field) => w.document.querySelector('[data-field="' + field + '"]');
  click("compose");
  await tick();
  q("body").value = "Typed text";
  q("body").dispatchEvent(new w.Event("input", { bubbles: true }));
  q("sender").value = "home";
  q("sender").dispatchEvent(new w.Event("change", { bubbles: true }));
  q("mode").value = "personal";
  q("to").value = "one@example.org,two@example.org";
  q("to").dispatchEvent(new w.Event("input", { bubbles: true }));
  await tick();
  assert.equal(q("body").value, "Typed text");
  assert.equal(q("signature").textContent, "signature-home");
  assert.match(q("preview").textContent, /2.*только свой адрес/);
  click("close");
  w.document.querySelector('[data-view="drafts"]').click();
  await tick();
  w.document.querySelector(".draftrow .thread").click();
  await tick();
  assert.equal(q("sender").value, "home");
  assert.equal(q("body").value, "Typed text");
  assert.equal(q("mode").value, "personal");
});
