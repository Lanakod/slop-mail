import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { addressBook } from "../ui/book.js";
import { recipientEditor, recipientFields } from "../ui/recipients.js";
import { snippetMenu } from "../ui/snippets.js";

test("contact card preserves search, opens the matched message and returns to the person", async () => {
  const w = new JSDOM(
    '<dialog id="dialog"><div id="content"></div></dialog><div class="readerhead"></div>',
  ).window;
  globalThis.document = w.document;
  const el = (id) => w.document.getElementById(id);
  const p = {
    id: "anna",
    name: "Анна",
    emails: "a@example.org, home@example.org",
    favorite: 1,
    labels: { "home@example.org": "Личный" },
  };
  let address = "",
    message;
  const book = addressBook({
    api: async (op, args) => {
      switch (op) {
        case "contacts":
          return [p, { id: "b", name: "Борис", emails: "b@example.org" }];
        case "groups":
          return [];
        case "contact_history":
          return [{ id: 15, thread: "chain", stamp: 20, subject: "Вопрос" }];
        case "contact":
          Object.assign(p, args);
          return p.id;
        default:
          throw Error(op);
      }
    },
    modal: (_, html) => (el("content").innerHTML = html),
    click: (id, fn) => (el(id).onclick = fn),
    action: (fn) => fn,
    shut: () => {},
    compose: (a) => (address = a),
    openThread: (t, id) => (message = [t, id]),
    date: () => "",
    onChange: () => {},
    profile: () => "work",
  });
  await book.open();
  el("booksearch").value = "анн";
  el("booksearch").oninput();
  await Promise.resolve();
  assert.equal(el("personlist").children.length, 1);
  await el("edit").onclick();
  el("pname").value = "Анна К.";
  await el("saveperson").onclick();
  assert.equal(el("booksearch").value, "анн");
  assert.equal(el("card").querySelector("h2").textContent, "Анна К.");
  await el("mailrows").children[1].onclick();
  assert.equal(address, "home@example.org");
  await el("personlist").firstChild.ondblclick();
  assert.equal(address, "a@example.org");
  await el("history").firstChild.onclick();
  assert.deepEqual(message, ["chain", 15]);
  await w.document.querySelector(".readerhead button").onclick();
  assert.equal(el("booksearch").value, "анн");
  await book.participant("HOME@example.org", "");
  assert.equal(el("card").querySelector("h2").textContent, "Анна К.");
  w.close();
});
test("recipient chips can be moved to blind copy and removed", async () => {
  const w = new JSDOM(
    "<main>" + recipientFields() + '<div data-field="suggest"></div></main>',
  ).window;
  globalThis.document = w.document;
  const host = w.document.querySelector("main");
  const value = recipientEditor(
    host,
    { to: "a@example.org" },
    [],
    async (_, args) => [args.to],
    () => {},
    (fn) => fn,
  );
  await Promise.resolve();
  const move = host.querySelector('[data-chips="to"] select');
  move.value = "bcc";
  move.onchange();
  assert.deepEqual(value(), { to: "", cc: "", bcc: "a@example.org" });
  host.querySelector('[data-chips="bcc"] button').click();
  assert.equal(value().bcc, "");
  w.close();
});
test("snippet inserts at the selected text", async () => {
  const w = new JSDOM('<textarea>Добрый !</textarea><div id="panel"></div>')
    .window;
  globalThis.document = w.document;
  const body = w.document.querySelector("textarea"),
    panel = w.document.querySelector("div");
  body.setSelectionRange(7, 7);
  await snippetMenu(
    panel,
    async () => [{ id: "g", name: "Приветствие", body: "день" }],
    (fn) => fn,
    (text) =>
      body.setRangeText(text, body.selectionStart, body.selectionEnd, "end"),
  );
  panel.querySelector(".snippet-list button").click();
  assert.equal(body.value, "Добрый день!");
  w.close();
});
