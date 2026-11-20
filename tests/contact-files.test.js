import test from "node:test";
import assert from "node:assert/strict";
import {
  writeContacts,
  readContacts,
  importPreview,
} from "../ui/contact-files.js";
const person = {
  name: 'Анна, "Тест"',
  emails: "work@example.org, home@example.org",
  labels: { "work@example.org": "Рабочий", "home@example.org": "Личный" },
};
for (const csv of [false, true])
  test("Address book round trip " + (csv ? "CSV" : "vCard"), () => {
    assert.deepEqual(readContacts(writeContacts([person], csv), csv), [person]);
  });
test("preview finds duplicates in both profile and input", () => {
  const result = importPreview(
    [person, person],
    [{ emails: "WORK@example.org" }],
  );
  assert.equal(result.items.length, 1);
  assert.equal(result.items[0].emails, "home@example.org");
  assert.equal(result.duplicates.length, 3);
});
