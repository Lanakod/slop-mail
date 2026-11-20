import { escapeHtml as h } from "./helpers.js";
export async function snippetMenu(panel, api, action, insert) {
  const rows = await api("snippets");
  if (!panel.isConnected) return;
  panel.hidden = false;
  panel.innerHTML =
    '<div class="snippet-list"></div><button class="add-snippet">＋ Новый шаблон</button><button class="hide-snippets">Закрыть</button>';
  const close = () => {
    panel.hidden = true;
  };
  panel.querySelector(".hide-snippets").onclick = close;
  const form = (row) => {
    panel.innerHTML = `<label>Название<input value="${h(row?.name)}"></label><label>Текст шаблона<textarea>${h(row?.body)}</textarea></label><div class="actions"><button>Сохранить</button><button>Назад</button></div>`;
    const [save, back] = panel.querySelectorAll("button");
    save.onclick = action(async () => {
      await api("save_snippet", {
        id: row?.id,
        name: panel.querySelector("input").value,
        body: panel.querySelector("textarea").value,
      });
      await snippetMenu(panel, api, action, insert);
    });
    back.onclick = action(() => snippetMenu(panel, api, action, insert));
  };
  panel.querySelector(".add-snippet").onclick = () => form(null);
  for (const row of rows) {
    const el = document.createElement("div");
    el.className = "actions";
    el.innerHTML = `<button>${h(row.name)}</button><button>Изменить</button><button>Удалить</button>`;
    const [use, edit, remove] = el.querySelectorAll("button");
    use.onclick = () => {
      insert(row.body);
      close();
    };
    edit.onclick = () => form(row);
    remove.onclick = action(async () => {
      if (await api("confirm", { text: "Удалить шаблон?" })) {
        await api("remove_snippet", { id: row.id });
        await snippetMenu(panel, api, action, insert);
      }
    });
    panel.querySelector(".snippet-list").append(el);
  }
}
