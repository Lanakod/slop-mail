import test from "node:test";
import assert from "node:assert/strict";
import { escapeHtml, splitQuote, basename, options } from "../ui/helpers.js";
test("quotes stay separate from reply text", () =>
  assert.deepEqual(splitQuote("Спасибо\n> Hi"), ["Спасибо\n", "> Hi"]));
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
