import test from "node:test";
import assert from "node:assert/strict";
import { escapeHtml, splitQuote, basename, options } from "../ui/helpers.js";
test("bottom-posted reply remains visible after the collapsed Russian quotation", () => {
  const quoted = "DCRDT писал(а) 2026-11-19 17:04:\nЦитата\n> Proverka chata";
  assert.deepEqual(
    splitQuote((quoted + "\n\nZebitls").replaceAll("\n", "\r\n")),
    [
      { quoted: true, text: quoted },
      { quoted: false, text: "\nZebitls" },
    ],
  );
});
test("inline replies keep their position between separate quotations", () => {
  assert.deepEqual(splitQuote("Спасибо\n> Hi\n\n>> Nested\nОтвет\n> Bye"), [
    { quoted: false, text: "Спасибо" },
    { quoted: true, text: "> Hi\n\n>> Nested" },
    { quoted: false, text: "Ответ" },
    { quoted: true, text: "> Bye" },
  ]);
});
test("attribution without marked quotation leaves authored text visible", () => {
  assert.deepEqual(splitQuote("On Thursday Jane wrote:\nHello"), [
    { quoted: false, text: "On Thursday Jane wrote:\nHello" },
  ]);
});
test("message labels are escaped", () =>
  assert.equal(escapeHtml("<script>"), "&lt;script&gt;"));
test("attachment names work on both platforms", () =>
  assert.equal(basename("C:\\mail\\счёт"), "счёт"));
test("sender selector preserves selected account", () =>
  assert.match(
    options(
      [
        ["a", "А"],
        ["b", "Б"],
      ],
      "b",
    ),
    /value="b" selected/,
  ));
