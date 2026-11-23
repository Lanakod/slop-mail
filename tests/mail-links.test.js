import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { chatBody, mailtoDraft } from "../ui/mail-links.js";

test("chat links preserve labels, escaped text and quoted URLs", () => {
  const dom = new JSDOM();
  globalThis.document = dom.window.document;
  const view = document.createElement("div");
  view.innerHTML = chatBody({
    body: "<img>\n> https://example.org/x?q=a&b=1.",
  });
  assert.equal(view.querySelector("img"), null);
  assert.equal(
    view.querySelector("details a").href,
    "https://example.org/x?q=a&b=1",
  );
  view.innerHTML = chatBody({
    body: "old fallback",
    html: '<p><a href="https://example.org/report">Отчёт</a> <a href="javascript:bad()">bad</a></p>',
  });
  assert.equal(view.querySelectorAll("a").length, 1);
  assert.equal(view.querySelector("a").textContent, "Отчёт");
  dom.window.close();
});

test("mailto actions show labels and open a prefilled draft from HTML or cached footnotes", () => {
  const dom = new JSDOM();
  globalThis.document = dom.window.document;
  const url =
    "mailto:ops@example.org?subject=ID+123&body=" +
    encodeURIComponent("Статус: Исполнено");
  for (const message of [
    { body: "", html: '<a href="' + url + '">Исполнить</a>' },
    {
      body: "[Исполнить][1]\n\n[1]: " + url.slice(0, 50) + "\n" + url.slice(50),
    },
  ]) {
    const host = document.createElement("div");
    host.innerHTML = chatBody(message);
    assert.equal(host.textContent, "Исполнить");
    assert.deepEqual(mailtoDraft(host.querySelector("a").href), {
      to: "ops@example.org",
      subject: "ID+123",
      body: "Статус: Исполнено",
    });
  }
  dom.window.close();
});
