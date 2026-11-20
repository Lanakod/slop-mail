import test from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";
import { chatBody } from "../ui/mail-links.js";

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
