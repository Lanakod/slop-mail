import { test } from "node:test";
import assert from "node:assert/strict";
import { JSDOM } from "jsdom";

test("settings persist a schedule and reusable connection template without credentials", async () => {
  const dom = new JSDOM("<main></main>", { url: "http://app.local" });
  const doc = dom.window.document;
  Object.assign(globalThis, {
    document: doc,
    localStorage: dom.window.localStorage,
  });
  const { settingsMenu, prefs, applyConfig, serverFields } = await import(
    "../ui/settings.js"
  );
  let rescheduled = 0;
  const menu = settingsMenu({
    modal: (_, body) => {
      doc.body.innerHTML = body;
    },
    click: (id, fn) => {
      doc.getElementById(id).onclick = fn;
    },
    api: async () => ({ version: "0.2.0" }),
    schedule: () => rescheduled++,
    flush: async () => {},
  });
  await menu();
  const el = (id) => doc.getElementById(id);
  el("minutes").value = "15";
  el("minutes").onchange();
  assert.equal(rescheduled, 1);
  el("newpreset").click();
  el("presetname").value = "Work";
  el("inhost").value = "mail.example.org";
  el("outhost").value = "relay.example.org";
  el("inlogin").value = "%EMAILLOCALPART%";
  await el("savepreset").onclick();
  const config = prefs.presets[0];
  assert.equal(
    JSON.parse(localStorage.getItem("mail-preferences")).minutes,
    15,
  );
  doc.body.innerHTML =
    serverFields("in", config.incoming) +
    serverFields("out", config.outgoing) +
    '<input id="inlogin"><input id="outlogin">';
  applyConfig(config, "alice@example.org");
  assert.equal(el("inlogin").value, "alice");
  assert.equal(el("outlogin").value, "alice@example.org");
  assert.equal(el("outhost").value, "relay.example.org");
  assert.deepEqual(Object.keys(config).sort(), [
    "id",
    "incoming",
    "login",
    "name",
    "outgoing",
    "smtp_login",
  ]);
  dom.window.close();
});
