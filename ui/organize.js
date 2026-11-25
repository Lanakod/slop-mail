import { escapeHtml as h } from "./helpers.js";
export function rulesPanel({ api, modal, click, action }) {
  async function show() {
    const rows = await api("rules");
    modal(
      "Правила для входящих",
      '<div id="rule-list"></div><label>Отправитель или домен<input id="rule-pattern" placeholder="example.org"></label><label>Метка<input id="rule-label" placeholder="Проект"></label><label>Переместить в папку<input id="rule-folder"></label><label class="check"><input type="checkbox" id="rule-quiet"> Без уведомлений</label><button id="rule-test">Предпросмотр</button><div id="rule-results" role="status"></div><button id="rule-save">Добавить правило</button>',
    );
    for (const r of rows) {
      const row = document.createElement("div");
      row.className = "entry";
      row.innerHTML =
        "<strong>" +
        h(r.pattern) +
        "</strong> · " +
        h(
          [r.label, r.destination, r.quiet ? "Без уведомлений" : ""]
            .filter(Boolean)
            .join(" · "),
        ) +
        " <button>Удалить</button>";
      row.querySelector("button").onclick = action(async () => {
        await api("drop_rule", { id: r.id });
        await show();
      });
      document.getElementById("rule-list").append(row);
    }
    /** @returns {HTMLInputElement} */
    const el = (id) =>
      /** @type {HTMLInputElement} */ (document.getElementById(id));
    const rule = () => ({
      pattern: el("rule-pattern").value,
      label: el("rule-label").value,
      folder: el("rule-folder").value,
      quiet: el("rule-quiet").checked,
    });
    const results = el("rule-results");
    let revision = 0;
    for (const id of [
      "rule-pattern",
      "rule-label",
      "rule-folder",
      "rule-quiet",
    ])
      el(id).oninput = () => {
        revision++;
        results.textContent = "";
      };
    click("rule-test", async () => {
      const current = ++revision,
        value = rule(),
        hits = await api("preview_rule", value);
      if (current !== revision || !results.isConnected) return;
      const effects = [
        value.label && "Метка: " + value.label,
        value.folder && "Папка: " + value.folder,
        value.quiet && "Без уведомлений",
      ]
        .filter(Boolean)
        .join(" · ");
      results.textContent = hits.length
        ? "До 10 совпадений среди последних 500 входящих. Действия: " + effects
        : "Среди последних 500 входящих совпадений нет.";
      for (const hit of hits) {
        const row = document.createElement("div");
        row.textContent = hit.sender + " · " + (hit.subject || "Без темы");
        results.append(row);
      }
    });
    click("rule-save", async () => {
      await api("save_rule", rule());
      await show();
    });
  }
  return show;
}
