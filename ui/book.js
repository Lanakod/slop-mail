import { escapeHtml as h, options } from "./helpers.js";
import { readContacts, writeContacts, importPreview } from "./contact-files.js";
export function addressBook({
  api,
  modal,
  click,
  action,
  shut,
  compose,
  openThread,
  date,
  onChange,
  profile,
}) {
  const $ = (id) => /** @type {any} */ (document.getElementById(id));
  let state = { profile: "", id: "", search: "", group: "", scroll: 0 },
    people = [],
    groups = [];
  const emails = (p) => p.emails.split(", ").filter(Boolean),
    selected = () => people.find((p) => p.id === state.id);
  async function refresh() {
    const active = profile();
    const rows = await Promise.all([api("contacts"), api("groups")]);
    if (active !== profile()) return false;
    [people, groups] = rows;
    people.sort(
      (a, b) =>
        Number(b.favorite) - Number(a.favorite) ||
        a.name.localeCompare(b.name, "ru"),
    );
    onChange(people);
    return true;
  }
  async function open(id = "") {
    if (state.profile !== profile())
      state = { profile: profile(), id: "", search: "", group: "", scroll: 0 };
    if (!(await refresh())) return;
    if (id) {
      state.id = id;
      state.search = "";
      state.group = "";
    }
    modal(
      "Адресная книга",
      `<div class="book"><section class="bookrail"><input id="booksearch" type="search" aria-label="Найти контакт" placeholder="Имя или email" value="${h(state.search)}"><select id="bookgroup">${options([["", "Все контакты"], ...groups.map((g) => [g.id, g.name])], state.group)}</select><div class="actions"><button id="newperson">＋ Контакт</button><button id="groups">Группы</button></div><div id="personlist"></div><div class="actions"><button id="importbook">Импорт…</button><button id="exportbook">Экспорт…</button></div></section><section id="card"></section></div>`,
    );
    $("dialog").classList.add("wide");
    $("booksearch").oninput = () => {
      state.search = $("booksearch").value;
      state.scroll = 0;
      draw();
    };
    $("bookgroup").onchange = () => {
      state.group = $("bookgroup").value;
      state.scroll = 0;
      draw();
    };
    click("newperson", () => edit(null));
    click("groups", manageGroups);
    click("importbook", importFile);
    click("exportbook", () => {
      modal(
        "Экспорт контактов",
        '<button id="vcf">vCard (.vcf)</button><button id="csv">CSV (.csv)</button>',
      );
      for (const fmt of ["vcf", "csv"])
        click(fmt, async () => {
          await api("export_book", {
            format: fmt,
            text: writeContacts(people, fmt === "csv"),
          });
          await open();
        });
    });
    draw();
  }
  function draw() {
    const list = $("personlist"),
      group = groups.find((g) => g.id === state.group);
    const query = state.search.trim().toLowerCase();
    const filtered = people.filter(
      (p) =>
        (!group || group.members.includes(p.id)) &&
        (p.name + " " + p.emails).toLowerCase().includes(query),
    );
    if (!filtered.some((p) => p.id === state.id))
      state.id = filtered[0]?.id || "";
    list.innerHTML =
      filtered
        .map(
          (p, i) =>
            `<button class="person" data-i="${i}" aria-pressed="${p.id === state.id}"><strong>${p.favorite ? "★ " : ""}${h(p.name || emails(p)[0])}</strong><small>${h(emails(p)[0])}</small></button>`,
        )
        .join("") || '<p class="note">Контакты не найдены</p>';
    list.scrollTop = state.scroll;
    list.onscroll = () => (state.scroll = list.scrollTop);
    for (const b of list.querySelectorAll("button")) {
      const p = filtered[+b.dataset.i];
      b.onclick = () => {
        state.id = p.id;
        for (const v of list.querySelectorAll("button"))
          v.setAttribute("aria-pressed", String(v === b));
        card().catch(showError);
      };
      b.ondblclick = action(() => compose(emails(p)[0]));
    }
    card().catch(showError);
  }
  function showError(e) {
    if ($("error")) $("error").textContent = String(e);
  }
  async function card() {
    const root = $("card"),
      p = selected();
    if (!p) {
      root.innerHTML =
        '<p class="note">Добавьте контакт или импортируйте адресную книгу.</p>';
      return;
    }
    root.innerHTML = `<h2>${h(p.name || emails(p)[0])}</h2><div class="actions"><button class="accent" id="write">Написать</button><button id="edit">Редактировать</button><button id="favorite">${p.favorite ? "★ В избранном" : "☆ В избранное"}</button><button id="remove">Удалить</button></div><div id="mailrows"></div><h3>Переписка</h3><div id="history">Загрузка…</div>`;
    click("write", () => compose(emails(p)[0]));
    click("edit", () => edit(p));
    click("favorite", async () => {
      await api("contact", { ...p, favorite: !p.favorite });
      await open();
    });
    click("remove", async () => {
      if (await api("confirm", { text: "Удалить контакт " + p.name + "?" })) {
        await api("remove_contact", { id: p.id });
        await open();
      }
    });
    emails(p).forEach((e, i) => {
      const b = document.createElement("button");
      b.className = "emailrow";
      b.textContent =
        (i === 0 ? "Основной · " : "") +
        (p.labels?.[e] ? p.labels[e] + " · " : "") +
        e +
        " ↗";
      b.onclick = action(() => compose(e));
      $("mailrows").append(b);
    });
    const node = $("history");
    const data = await api("contact_history", { id: p.id });
    if (!node.isConnected) return;
    node.innerHTML =
      data
        .map(
          (m, i) =>
            `<button class="emailrow" data-i="${i}">${h(m.subject || "Без темы")} · ${h(date(m.stamp))}</button>`,
        )
        .join("") || "Пока нет писем";
    for (const b of node.querySelectorAll("button"))
      b.onclick = action(async () => {
        const m = data[+b.dataset.i];
        shut();
        await openThread(m.thread, m.id);
        const back = document.createElement("button");
        back.textContent = "← К контакту";
        back.onclick = action(() => open());
        document.querySelector(".readerhead")?.prepend(back);
      });
  }
  function edit(p, extra = "", name = "") {
    const values = p ? emails(p) : [];
    if (extra && !values.includes(extra)) values.push(extra);
    if (!values.length) values.push("");
    modal(
      p ? "Редактировать контакт" : "Новый контакт",
      `<label>Имя<input id="pname" value="${h(p?.name || name)}"></label><div id="editmails"></div><button id="addmail">＋ Email</button><div class="actions"><button class="accent" id="saveperson">Сохранить</button><button id="cancelbook">Назад</button></div>`,
    );
    function row(e, primary) {
      const div = document.createElement("div");
      div.className = "addrrow";
      div.innerHTML = `<label>Основной<input type="radio" name="primary" ${primary ? "checked" : ""}></label><label>Email<input type="email" value="${h(e)}"></label><label>Метка<input type="text" placeholder="Рабочий" value="${h(p?.labels?.[e] || "")}"></label><button aria-label="Убрать адрес">×</button>`;
      div.querySelector("button").onclick = () => div.remove();
      $("editmails").append(div);
    }
    values.forEach((e, i) => row(e, i === 0));
    click("addmail", () => row("", false));
    click("cancelbook", () => open());
    click("saveperson", async () => {
      const rows = Array.from(
        $("editmails").children,
        /** @param {HTMLElement} r */ (r) => ({
          email: /** @type {HTMLInputElement} */ (
            r.querySelector("input[type=email]")
          ).value.trim(),
          label: /** @type {HTMLInputElement} */ (
            r.querySelector("input[type=text]")
          ).value.trim(),
          primary: /** @type {HTMLInputElement} */ (
            r.querySelector("input[type=radio]")
          ).checked,
        }),
      )
        .filter((r) => r.email)
        .sort((a, b) => Number(b.primary) - Number(a.primary));
      if (rows.some((r) => r.email.includes(",") || r.email.includes(";")))
        throw Error("Укажите один email в каждой строке");
      state.id = await api("contact", {
        id: p?.id || "",
        name: $("pname").value.trim(),
        emails: rows.map((r) => r.email).join(", "),
        labels: Object.fromEntries(rows.map((r) => [r.email, r.label])),
        favorite: !!p?.favorite,
      });
      if (!p) state.search = state.group = "";
      await open();
    });
  }
  async function participant(email, name) {
    if (!(await refresh())) return;
    const existing = people.find((p) =>
      emails(p).some((e) => e.toLowerCase() === email.toLowerCase()),
    );
    if (existing) return open(existing.id);
    modal(
      email,
      `<button id="addnew">Создать контакт</button><label>Существующий контакт<select id="existing">${options(
        people.map((p) => [p.id, p.name || p.emails]),
        null,
      )}</select></label><button id="append" ${people.length ? "" : "disabled"}>Добавить адрес к контакту</button>`,
    );
    click("addnew", () => edit(null, email, name));
    click("append", () =>
      edit(
        people.find((p) => p.id === $("existing").value),
        email,
      ),
    );
  }
  function manageGroups() {
    modal(
      "Группы контактов",
      '<div id="grouprows"></div><div class="actions"><button id="newgroup">＋ Группа</button><button id="groupback">Назад</button></div>',
    );
    for (const g of groups) {
      const row = document.createElement("div");
      row.className = "entry";
      row.innerHTML = `<strong>${h(g.name)}</strong> · ${g.members.length}<div class="actions"><button>Написать группе</button><button>Изменить</button><button>Удалить</button></div>`;
      const [write, change, remove] = row.querySelectorAll("button");
      write.disabled = !g.members.length;
      write.onclick = action(() =>
        compose(
          people
            .filter((p) => g.members.includes(p.id))
            .map((p) => emails(p)[0])
            .join(", "),
        ),
      );
      change.onclick = () => editGroup(g);
      remove.onclick = action(async () => {
        if (await api("confirm", { text: "Удалить группу " + g.name + "?" })) {
          await api("remove_group", { id: g.id });
          await refresh();
          state.group = "";
          manageGroups();
        }
      });
      $("grouprows").append(row);
    }
    click("newgroup", () => editGroup(null));
    click("groupback", () => open());
  }
  function editGroup(g) {
    modal(
      "Группа",
      `<label>Название<input id="gname" value="${h(g?.name)}"></label><div id="members">${people.map((p, i) => `<label class="check"><input type="checkbox" data-i="${i}" ${g?.members.includes(p.id) ? "checked" : ""}>${h(p.name)} · ${h(emails(p)[0])}</label>`).join("")}</div><button id="gsave">Сохранить группу</button>`,
    );
    click("gsave", async () => {
      await api("save_group", {
        id: g?.id,
        name: $("gname").value,
        members: Array.from(
          $("members").querySelectorAll(":checked"),
          (b) => people[b.dataset.i].id,
        ),
      });
      await refresh();
      manageGroups();
    });
  }
  function importFile() {
    modal(
      "Импорт контактов",
      '<label>Выберите vCard или CSV<input id="bookfile" type="file" accept=".vcf,.vcard,.csv"></label><div id="review"></div><button id="importok" disabled>Импортировать</button>',
    );
    let items = [];
    $("bookfile").onchange = action(async () => {
      const file = $("bookfile").files[0];
      $("importok").disabled = true;
      if (!file) return;
      const preview = importPreview(
        readContacts(await file.text(), /\.csv$/i.test(file.name)),
        people,
      );
      items = preview.items;
      $("review").innerHTML =
        `<strong>Будет добавлено: ${items.length}</strong><p>Уже есть или повторяются: ${h(preview.duplicates.join(", ") || "—")}</p><p>Некорректные адреса: ${h(preview.invalid.join(", ") || "—")}</p>${items.map((p) => `<div>${h(p.name)} · ${h(p.emails)}</div>`).join("")}`;
      $("importok").disabled = !items.length;
    });
    click("importok", async () => {
      await api("import_contacts", { items });
      await open();
    });
  }
  return { open, participant };
}
