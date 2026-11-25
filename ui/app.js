import { htmlNotice } from "./html-notice.js";
import { accountIcon, iconOptions } from "./account-icons.js";
import { diskReport } from "./disk-usage.js";
import { updateChecks } from "./updates.js";
import { richEditor } from "./rich-editor.js";
import { chatBody, mailtoDraft } from "./mail-links.js";
import { rulesPanel } from "./organize.js";
import { threadList } from "./thread-list.js";
import { addressBook } from "./book.js";
import { recipientEditor, recipientFields } from "./recipients.js";
import { snippetMenu } from "./snippets.js";
import {
  prefs,
  persist,
  settingsMenu,
  serverFields,
  bindPorts,
  endpoint,
  applyConfig,
} from "./settings.js";
import { escapeHtml as h, options, basename } from "./helpers.js";
/** @returns {any} */
const $ = (id) => document.getElementById(id);
const api = (op, args = {}, profile = pid) =>
  window.__TAURI__.core.invoke("request", { op, args, profile });
const composeContext = window.__composeContext;
let pid = "",
  accounts = [],
  folders = [],
  people = [],
  listVersion = 0,
  readerVersion = 0;
let selection = { account: "", folder: "" },
  syncProfile = "";
let quickView = "";
let savedSearches = [];
let draftWrites = Promise.resolve();
const ask = (text) => api("confirm", { text });
const status = (t) => {
  $("status").textContent = t;
  $("status").title = t;
};
const err = (e) => {
  status(String(e));
  if ($("dialog").open) {
    let el = $("error");
    if (el) el.textContent = String(e);
  }
};
const action = (fn) => async (e) => {
  try {
    await fn(e);
  } catch (error) {
    err(error);
  }
};
const click = (id, fn) => ($(id).onclick = action(fn));
function modal(title, content) {
  $("dialog").classList.remove("wide");
  $("title").textContent = title;
  $("content").innerHTML = content + '<div id="error" role="alert"></div>';
  if (!$("dialog").open) $("dialog").showModal();
}
function shut() {
  $("dialog").close();
}
function empty(text) {
  return '<div class="empty"><b>✉</b><h2>' + h(text) + "</h2></div>";
}
function date(stamp) {
  return new Date(stamp * 1000).toLocaleString("ru", {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}
const accountOptions = (v) =>
  options(
    accounts.map((a) => [a.id, a.name + " · " + a.address]),
    v,
  );
function profileDialog() {
  modal(
    "Профили",
    '<label>Название нового профиля<input id="pname" placeholder="Работа" autofocus></label><div class="actions"><button id="pdefault" class="accent">Создать в стандартной папке</button><button id="pcustom">Выбрать папку…</button></div><hr><button id="popen">Открыть существующий профиль…</button><button id="prestore">Восстановить копию…</button>',
  );
  click("prestore", restoreBackup);
  for (const [button, fresh, def] of [
    ["pdefault", true, true],
    ["pcustom", true, false],
    ["popen", false, false],
  ])
    click(button, async () => {
      const name = $("pname").value.trim();
      if (fresh && !name) throw Error("Введите название профиля");
      const p = await api("profile", { new: fresh, default: def, name });
      if (p) {
        shut();
        await loadProfiles(p.id);
      }
    });
}
async function restoreBackup() {
  const profile = await api("restore_backup");
  if (profile) {
    shut();
    await loadProfiles(profile.id);
  }
}
async function loadProfiles(prefer) {
  let profiles = await api("profiles");
  pid = profiles.find((p) => p.id === prefer)?.id || profiles[0]?.id || "";
  $("profile").innerHTML = options(
    profiles.map((p) => [p.id, p.name]),
    pid,
  );
  await switchProfile();
}
async function switchProfile() {
  await draftWrites;
  pid = $("profile").value;
  const active = pid;
  $("undo-actions").replaceChildren();
  schedule();
  localStorage.setItem("profile", pid);
  quickView = "";
  mailList.reset();
  activeMessage = null;
  readerVersion++;
  listVersion++;
  $("threads").innerHTML = empty("Загрузка почты…");
  $("folders").replaceChildren();
  $("filters").replaceChildren();
  $("addresses").replaceChildren();
  selection = { account: "", folder: "" };
  $("search").value = "";
  $("reader").innerHTML = empty("Выберите переписку");
  document.body.classList.remove("reading");
  accounts = [];
  folders = [];
  people = [];
  for (const id of ["compose", "sync", "queue", "contacts", "add"])
    $(id).disabled = !pid;
  if (!pid) {
    $("threads").innerHTML = empty("Профиль не выбран");
    status("Создайте профиль или откройте папку с почтой");
    profileDialog();
    return;
  }
  status("Открытие профиля…");
  await api("resume_work", {}, active);
  const data = await Promise.all(
    [
      "accounts",
      "folders",
      "contacts",
      "saved_searches",
      "mailbox_display",
    ].map((op) => api(op, {}, active)),
  );
  if (pid !== active) return;
  [accounts, folders, people, savedSearches, mailboxDisplay] = data;
  renderNav();
  renderFilters();
  await list();
  status(
    accounts.length ? "Локальная почта открыта" : "Подключите первый ящик",
  );
  if (accounts.length) {
    background(active, prefs.startup);
  }
}
const views = {
  snoozed: "◷ Отложенные",
  unread: "● Непрочитанные",
  starred: "★ Со звездой",
  files: "▧ С вложениями",
  drafts: "✎ Черновики",
};
let mailboxDisplay = [];
const liveSync = new Set(),
  backgroundSync = new Set();
let nextPoll = 0;
if (!composeContext) {
  window.__TAURI__.event
    .listen(
      "mail-progress",
      action(async ({ payload }) => {
        if (payload.profile !== pid) return;
        const key = pid + "/" + payload.account;
        if (payload.busy) backgroundSync.add(key);
        else backgroundSync.delete(key);
        if (payload.busy) renderNav();
        else await refreshFolders();
      }),
    )
    .catch(err);
  window.__TAURI__.event
    .listen(
      "mail-next-poll",
      action(({ payload }) => {
        if (payload.profile === pid) {
          nextPoll = payload.stamp;
          renderNav();
        }
      }),
    )
    .catch(err);
}
function renderNav() {
  let html = '<button data-index="-1">▤　Вся почта</button>';
  for (const [id, label] of Object.entries(views)) {
    html += `<button data-view="${id}" class="${quickView === id ? "active" : ""}">${h(label)}</button>`;
  }
  for (const [i, item] of savedSearches.entries())
    html += '<button data-query="' + i + '">⌕ ' + h(item.query) + "</button>";
  const totals = accounts.map((a) =>
    folders
      .filter((f) => f.account === a.id)
      .reduce((n, f) => n + (f.unread || 0), 0),
  );
  const badge = (n) => (n ? '<span class="unread-count">' + n + "</span>" : "");
  window.__TAURI__.core
    .invoke("tray_counts", {
      summary:
        "Slop Mail · " +
        (accounts
          .map((a, i) => (a.name || a.address) + " — " + totals[i])
          .join(", ") || "Нет ящиков"),
    })
    .catch(err);
  accounts.forEach((a, i) => {
    const details = mailboxDisplay.find((row) => row.account === a.id);
    const next =
      prefs.minutes && nextPoll
        ? "Следующий опрос: " + date(nextPoll)
        : prefs.minutes
          ? "Автопроверка включена"
          : "Только вручную";
    const syncText =
      liveSync.has(pid + "/" + a.id) || backgroundSync.has(pid + "/" + a.id)
        ? "Получение…"
        : details?.problem ||
          (details?.last_ok
            ? "Получено " + date(details.last_ok)
            : "Ещё не проверялся");
    html += `<div class="accountname"><button data-account="${i}" title="${h(a.address)}">${accountIcon(details?.icon)}　${h(a.name || a.address)} ${badge(totals[i])}</button><button data-edit="${i}" title="Настройки ящика">⚙</button></div><small class="mailbox-state" title="${h([details?.last_ok ? "Последний успех: " + date(details.last_ok) : "", next].filter(Boolean).join(" · "))}">${h(syncText)}</small>`;
    folders.forEach((f, j) => {
      if (f.account === a.id)
        html += `<button class="folder" data-index="${j}">▱　${h(f.name)} ${badge(f.unread)}</button>`;
    });
  });
  $("folders").innerHTML = html;
  for (const b of $("folders").querySelectorAll("[data-query]"))
    b.onclick = action(async () => {
      quickView = "";
      selection = { account: "", folder: "" };
      $("search").value = savedSearches[Number(b.dataset.query)].query;
      renderFilters();
      renderNav();
      await list();
    });
  for (const b of $("folders").querySelectorAll("[data-view]"))
    b.onclick = action(async () => {
      quickView = b.dataset.view;
      selection = { account: "", folder: "" };
      $("search").value = "";
      document.body.classList.remove("navopen", "reading");
      renderNav();
      renderFilters();
      await list();
    });
  $("folders")
    .querySelectorAll("[data-edit]")
    .forEach(
      (b) => (b.onclick = () => accountDialog(accounts[+b.dataset.edit])),
    );
  $("folders")
    .querySelectorAll("[data-index],[data-account]")
    .forEach((b) => {
      const f =
        b.dataset.account !== undefined
          ? { account: accounts[+b.dataset.account].id, folder: "" }
          : folders[+b.dataset.index]
            ? {
                account: folders[+b.dataset.index].account,
                folder: folders[+b.dataset.index].name,
              }
            : { account: "", folder: "" };
      b.classList.toggle(
        "active",
        !quickView &&
          selection.account === f.account &&
          selection.folder === f.folder,
      );
      b.onclick = action(async () => {
        quickView = "";
        selection = f;
        $("search").value = "";
        document.body.classList.remove("navopen", "reading");
        renderFilters();
        renderNav();
        await list();
      });
    });
  $("addresses").innerHTML = people
    .flatMap((p) =>
      p.emails
        .split(", ")
        .map((e) => `<option value="${h(e)}">${h(p.name)}</option>`),
    )
    .join("");
}
function renderFilters() {
  const el = $("filters");
  el.innerHTML = `<label>Ящик<select id="fa">${options([["", "Все ящики"], ...accounts.map((a) => [a.id, a.address])], selection.account)}</select></label><label>Папка<select id="ff">${options([["", "Все папки"], ...Array.from(new Set(folders.map((f) => f.name))).map((f) => [f, f])], selection.folder)}</select></label><label>С даты<input type="date" id="from"></label><label>По дату<input type="date" id="until"></label><label class="check"><input type="checkbox" id="hasfiles"> С вложениями</label><button id="save-query">Сохранить запрос</button><button id="drop-query">Убрать сохранённый запрос</button>`;
  for (const [id, op] of [
    ["save-query", "save_search"],
    ["drop-query", "drop_search"],
  ])
    click(id, async () => {
      await api(op, { q: $("search").value });
      savedSearches = await api("saved_searches");
      renderNav();
    });
  el.onchange = action(list);
}
let activeMessage = null;
const mailList = threadList({
  api,
  action,
  date,
  open: openThread,
  refresh: refreshMail,
  bulk,
});
async function list() {
  if (!pid) return;
  mailList.closeMenu();
  const version = ++listVersion,
    q = $("search").value;
  let acc = $("fa")?.value || "",
    folder = $("ff")?.value || "";
  if (quickView === "drafts") {
    await draftWrites;
    const rows = await api("drafts");
    if (version !== listVersion) return;
    const drafts = rows
      .map((r) => ({
        ...JSON.parse(r.data),
        slot: r.slot,
        modified: r.modified,
      }))
      .filter((d) =>
        [d.to, d.cc, d.bcc, d.subject, d.body]
          .join(" ")
          .toLowerCase()
          .includes(q.toLowerCase()),
      );
    $("foldertitle").textContent = "Черновики";
    $("count").textContent = drafts.length;
    $("threads").innerHTML =
      drafts
        .map(
          (d) =>
            `<div class="draftrow"><button class="thread"><time>${d.modified ? h(date(d.modified)) : ""}</time><strong>${h(d.subject || "Без темы")}</strong><span class="meta">${h([d.to, d.cc, d.bcc].filter(Boolean).join(", ") || "Без получателей")}</span><small>${h(d.body)}</small></button><button class="dropdraft" aria-label="Удалить черновик">×</button></div>`,
        )
        .join("") || empty("Черновиков нет");
    $("threads")
      .querySelectorAll(".draftrow")
      .forEach((el, i) => {
        const d = drafts[i];
        el.querySelector(".thread").onclick = action(async () => {
          if (
            d.slot.startsWith("reply:") &&
            (await openThread(d.slot.slice(6), d.parent))
          )
            return;
          await compose("", d.slot);
        });
        el.querySelector(".dropdraft").onclick = action(async () => {
          if (await ask("Удалить черновик?")) {
            await api("remove_draft", { slot: d.slot });
            await list();
          }
        });
      });
    return;
  }
  const filters = {
    grouped: true,
    view: quickView,
    q,
    sort: prefs.sort,
    account: acc,
    folder,
    from: $("from")?.value,
    until: $("until")?.value,
    files: $("hasfiles")?.checked,
  };
  const data = await api("list", filters);
  if (version !== listVersion) return;
  $("foldertitle").textContent = q
    ? "Результаты поиска"
    : views[quickView] ||
      folder ||
      accounts.find((a) => a.id === acc)?.name ||
      "Вся почта";
  $("count").textContent = data.length;
  if (data.length)
    mailList.render($("threads"), data, filters, activeMessage, pid);
  else
    $("threads").innerHTML = empty(
      q ? "Ничего не найдено" : "В папке пока нет писем",
    );
}
async function openThread(key, hit, replyAll = false) {
  await draftWrites;
  const token = ++readerVersion;
  activeMessage = hit;
  const data = await api("thread", { thread: key, hit, summary: true });
  if (token !== readerVersion) return;
  const last = data.at(-1);
  if (!last) return false;
  document.body.classList.add("reading");
  $("reader").innerHTML =
    `<div class="readerhead"><button id="back">← Список</button><h1>${h(last.subject || "Без темы")}</h1></div><div id="timeline" class="timeline"></div><div id="quick"></div>`;
  click("back", () => document.body.classList.remove("reading"));
  function bubble(m) {
    const files = m.files;
    if (token !== readerVersion) return;
    const body = m.html ? "" : chatBody(m);
    let art = document.createElement("article");
    art.className =
      "bubble" +
      (m.outgoing ? " out" : "") +
      (m.id === hit && $("search").value ? " hit" : "");
    art.id = "msg" + m.id;
    art.innerHTML = `<strong>${h(m.sender)}</strong><div class="meta">Кому: ${h(m.recipients)} · ${h(date(m.stamp))}</div><div class="plain-body">${body}</div><div class="parts"></div><div class="actions"></div>`;
    for (const anchor of art.querySelectorAll("a"))
      anchor.onclick = action(async (e) => {
        e.preventDefault();
        await followLink(anchor.href);
      });
    const btn = (parent, text, fn) => {
      let b = document.createElement("button");
      b.textContent = text;
      b.onclick = action(fn);
      parent.append(b);
    };
    for (const [selector, participants] of [
      ["strong", m.authors],
      [".meta", m.addressees],
    ]) {
      const node = art.querySelector(selector);
      if (participants.length) {
        node.replaceChildren();
        if (selector === ".meta") node.append("Кому: ");
        participants.forEach((p, i) => {
          if (i) node.append(", ");
          btn(node, p.name || p.email, () =>
            book.participant(p.email, p.name || ""),
          );
          node.lastElementChild.title = p.email;
        });
        if (selector === ".meta") node.append(" · " + date(m.stamp));
      }
    }
    if (m.outgoing) {
      const stamp = document.createElement("small");
      stamp.className = "meta";
      stamp.textContent = m.copy_folder
        ? "Сохранено на сервере · " + m.copy_folder
        : m.copy_pending
          ? "Ожидает копирования на сервер"
          : m.folder === "Отправленные (локально)"
            ? "Отправлено · локальная копия"
            : "На сервере · " + m.folder;
      art.append(stamp);
    }
    let actions = art.querySelector(".actions");
    btn(
      actions,
      m.starred ? "★ Убрать звезду" : "☆ Поставить звезду",
      async () => {
        await api("star", { id: m.id, on: !m.starred });
        await openThread(key, m.id);
      },
    );
    if (m.html) {
      const host = art.querySelector(".plain-body");
      htmlNotice(host, (remote) => htmlView(art, m, files, remote), action);
      if (!m.outgoing)
        btn(actions, "Обновить HTML с сервера", async () => {
          await api("restore_links", { id: m.id });
          await openThread(key, m.id);
        });
    }
    for (const f of files) {
      let el = document.createElement("div");
      el.className = "entry";
      el.textContent = f.name + " · " + f.size.toLocaleString("ru") + " Б ";
      for (const [label, type] of [
        ["Открыть", "open"],
        ["Открыть с помощью…", "choose"],
        ["Сохранить как…", "save"],
      ])
        btn(el, label, () => api("file", { id: f.id, action: type }));
      art.querySelector(".parts").append(el);
    }
    btn(actions, "Переслать", async () => {
      const slot = "new:" + crypto.randomUUID();
      const draft = {
        slot,
        account: m.account,
        to: "",
        cc: "",
        bcc: "",
        subject: "Fwd: " + m.subject,
        body:
          "\n\nПересылаемое сообщение\nОт: " +
          m.sender +
          "\nТема: " +
          m.subject +
          "\n\n" +
          m.body,
        files: [],
        storedFiles: files.map(({ id, name }) => ({ id, name })),
      };
      await api("save_draft", { slot, data: draft });
      await compose("", slot);
    });
    btn(actions, "Ответить", () => composer($("quick"), m, false, true));
    btn(actions, "Ответить всем", () => composer($("quick"), m, true, true));
    btn(actions, "Удалить локально", async () => {
      if (!(await ask("Удалить локальную копию письма?"))) return;
      await api("delete", { id: m.id });
      await list();
      if (data.length > 1) await openThread(key, null);
      else {
        $("reader").innerHTML = empty("Выберите переписку");
        document.body.classList.remove("reading");
      }
    });
    return art;
  }
  const timeline = $("timeline");
  const focus = hit ? data.findIndex((m) => m.id === hit) : data.length - 1;
  let start = Math.max(0, Math.min(focus - 20, data.length - 40)),
    end = Math.min(start + 40, data.length);
  const earlier = document.createElement("button"),
    later = document.createElement("button");
  earlier.textContent = "Предыдущие письма";
  later.textContent = "Следующие письма";
  timeline.append(earlier, later);
  function buttons() {
    earlier.hidden = start === 0;
    later.hidden = end === data.length;
  }
  async function page(from, to, before) {
    const items = data.slice(from, to);
    const rows = await api("thread_bodies", {
      thread: key,
      ids: items.map((m) => m.id),
    });
    if (token !== readerVersion) return;
    const bodies = new Map(rows.map((row) => [row.id, row]));
    const fragment = document.createDocumentFragment();
    for (const m of items)
      if (bodies.has(m.id))
        fragment.append(bubble({ ...m, ...bodies.get(m.id) }));
    timeline.insertBefore(fragment, before);
  }
  earlier.onclick = action(async () => {
    earlier.disabled = true;
    try {
      const next = Math.max(0, start - 40);
      const anchor = earlier.nextElementSibling;
      const top = anchor.getBoundingClientRect().top;
      await page(next, start, anchor);
      start = next;
      buttons();
      timeline.scrollTop += anchor.getBoundingClientRect().top - top;
    } finally {
      earlier.disabled = false;
    }
  });
  later.onclick = action(async () => {
    later.disabled = true;
    try {
      const next = Math.min(data.length, end + 40);
      await page(end, next, later);
      end = next;
      buttons();
    } finally {
      later.disabled = false;
    }
  });
  buttons();
  await page(start, end, later);
  if (token !== readerVersion) return;
  const recipientMessage = replyAll ? data.find((m) => m.id === hit) : last;
  await composer($("quick"), recipientMessage || last, replyAll, replyAll);
  if (token !== readerVersion) return;
  const target = $("msg" + (hit || last.id));
  target?.scrollIntoView({ block: "nearest" });
  await refreshMail();
  return true;
}
async function htmlView(art, m, files, remote) {
  const template = document.createElement("template");
  template.innerHTML = m.html;
  let hasRemoteImages = false;
  for (const img of template.content.querySelectorAll("img")) {
    const src = img.getAttribute("src") || "";
    img.removeAttribute("src");
    if (src.startsWith("cid:")) {
      const f = files.find((f) => f.cid === src.slice(4));
      if (f) img.src = await api("inline", { id: f.id });
    } else if (/^https:\/\//i.test(src)) {
      hasRemoteImages = true;
      if (remote) img.src = src;
    }
  }
  art.querySelector(".html-links")?.remove();
  const links = document.createElement("div");
  links.className = "html-links";
  for (const a of template.content.querySelectorAll("a")) {
    const url = a.getAttribute("href");
    a.removeAttribute("href");
    if (!/^(https?:\/\/|mailto:)/i.test(url || "")) continue;
    const link = document.createElement("a");
    link.href = url;
    link.textContent = a.textContent || url;
    link.title = url;
    link.onclick = action(async (e) => {
      e.preventDefault();
      await followLink(url);
    });
    links.append(link, " ");
  }
  let frame = art.querySelector("iframe") || document.createElement("iframe");
  frame.setAttribute("sandbox", "");
  frame.title = "Содержимое письма";
  frame.referrerPolicy = "no-referrer";
  frame.srcdoc =
    `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data: ${remote ? "https:" : ""}; style-src 'unsafe-inline'"><meta name="referrer" content="no-referrer"><style>html{color-scheme:light}body{margin:12px;color:#202623;background:white;font:15px/1.45 system-ui;overflow-wrap:anywhere}table{max-width:100%}img{max-width:100%;height:auto}</style>` +
    template.innerHTML;
  const host = art.querySelector(".plain-body");
  host.replaceChildren(frame, links);
  return hasRemoteImages;
}
function accountDialog(edit) {
  const a = edit || {
    name: "",
    address: "",
    login: "",
    smtp_login: "",
    signature: "",
    id: "",
    keep: true,
    incoming: { proto: "imap", tls: "tls", host: "", port: 993 },
    outgoing: { proto: "smtp", tls: "tls", host: "", port: 465 },
  };
  const input = (id, label, v, type = "text") =>
    `<label>${label}<input id="${id}" type="${type}" value="${h(v)}"></label>`;
  const server = (k, title, s) =>
    `<fieldset><legend>${title}</legend>${serverFields(k, s)}<div class="grid">${input(k + "login", "Логин", k === "in" ? a.login : a.smtp_login)}${input(k + "pass", "Пароль", "", "password")}</div><button id="test${k}">Проверить соединение</button></fieldset>`;
  modal(
    edit ? "Настройки ящика" : "Подключить ящик",
    `<label>Иконка ящика · Font Awesome<select id="mail-icon">${options(iconOptions, mailboxDisplay.find((row) => row.account === a.id)?.icon || "envelope")}</select></label><div class="grid">${input("aname", "Имя отправителя", a.name)}${input("address", "Email", a.address, "email")}</div><div class="actions"><button id="discover">Найти настройки по email</button><select id="applypreset" aria-label="Применить шаблон">${options([["", "Шаблон подключения…"], ...prefs.presets.map((p) => [p.id, p.name])], "")}</select></div>${server("in", "Входящая почта", a.incoming)}<label class="check"><input id="keep" type="checkbox" ${a.keep ? "checked" : ""}> POP3 · Оставлять письма на сервере</label>${server("out", "Исходящая почта", a.outgoing)}<label class="check"><input id="remember" type="checkbox" ${localStorage.getItem("remember:" + pid + ":" + a.id) !== "false" ? "checked" : ""}> Запомнить пароли на этом компьютере</label><label>Подпись<textarea id="signature">${h(a.signature)}</textarea></label><button id="saveaccount" class="accent">Сохранить ящик</button>`,
  );
  bindPorts();
  const dialogNode = $("discover");
  click("discover", async () => {
    const address = $("address").value.trim();
    dialogNode.disabled = true;
    $("error").textContent = "Поиск настроек…";
    try {
      const config = await api("discover", { address });
      if (dialogNode.isConnected && $("address").value.trim() === address) {
        applyConfig(config, address);
        $("error").textContent = "Источник: " + config.source;
      }
    } finally {
      dialogNode.disabled = false;
    }
  });
  $("applypreset").onchange = () => {
    const config = prefs.presets.find((p) => p.id === $("applypreset").value);
    if (config) applyConfig(config, $("address").value.trim());
  };
  const data = () => {
    return {
      account: {
        id: a.id,
        name: $("aname").value,
        address: $("address").value.trim(),
        login: $("inlogin").value,
        smtp_login: $("outlogin").value,
        signature: $("signature").value,
        keep: $("keep").checked,
        incoming: endpoint("in"),
        outgoing: endpoint("out"),
      },
      icon: $("mail-icon").value,
      remember: $("remember").checked,
      in: $("inpass").value,
      out: $("outpass").value,
    };
  };
  for (const side of ["in", "out"])
    click("test" + side, async () => {
      $("error").textContent = "Подключение…";
      const result = await api("test", { ...data(), side });
      $("error").textContent =
        result.idle === true
          ? "Соединение установлено. Сервер поддерживает IMAP IDLE."
          : result.idle === false
            ? "Соединение установлено. Сервер предлагает проверку по расписанию."
            : "Соединение установлено";
    });
  click("saveaccount", async () => {
    const args = data();
    const id = await api("account", args);
    localStorage.setItem("remember:" + pid + ":" + id, String(args.remember));
    shut();
    await switchProfile();
  });
}
async function composer(
  host,
  parent,
  all,
  retarget,
  initialTo = "",
  newSlot = "",
) {
  if (!accounts.length) {
    accountDialog(null);
    return;
  }
  const slot = parent ? "reply:" + parent.thread : newSlot,
    active = pid;
  await draftWrites;
  let draft = await api("draft", { slot });
  if (!host.isConnected || active !== pid) return;
  let sender = draft?.account || parent?.account || accounts[0].id;
  let signature = accounts.find((a) => a.id === sender)?.signature || "";
  let to =
    parent && (!draft || retarget)
      ? await api("reply", { id: parent.id, all })
      : draft?.to || initialTo;
  if (!host.isConnected || active !== pid) return;
  const stamps = new Map(Object.entries(draft?.fileStamps || {}));
  const issues = new Map();
  const storedFiles = [...(draft?.storedFiles || [])];
  let files = draft?.files || [],
    parentId = retarget ? parent?.id : draft?.parent || parent?.id || null;
  const q = (k) => host.querySelector('[data-field="' + k + '"]');
  host.innerHTML = `<div class="composer">
    <div class="actions compose-actions"><button type="button" class="accent" data-field="send">Отправить ↗</button><button type="button" data-field="attach">＋ Вложение</button></div>
    ${
      parent
        ? ""
        : `<div class="line"><label>Отправитель<select data-field="sender">${accountOptions(sender)}</select></label>
    <label>Режим<select data-field="mode">${options(
      [
        ["group", "Общее письмо"],
        ["personal", "Персональная рассылка"],
      ],
      draft?.personal ? "personal" : "group",
    )}</select></label></div>
    ${recipientFields()}<div data-field="suggest" class="actions"></div><label>Тема<input data-field="subject" value="${h(draft?.subject || composeContext?.subject || "")}"></label>`
    }
    <div class="format-tools" role="toolbar" aria-label="Форматирование"></div>
    <form class="link-form" hidden><label>Адрес ссылки<input type="url" required placeholder="https://"></label><button>Вставить</button><button type="button">Отмена</button></form>
    <div contenteditable="true" role="textbox" aria-multiline="true" aria-label="Текст письма" data-field="body" class="rich-body"></div>
    <button data-field="templates">Шаблоны ответов</button><div data-field="snippets" hidden></div><div class="signature" data-field="signature"></div><div data-field="files"></div><div class="hint" data-field="preview"></div></div>`;
  const initial = {
    to,
    cc: retarget ? "" : draft?.cc || "",
    bcc: retarget ? "" : draft?.bcc || "",
  };
  const values = parent
    ? () => initial
    : recipientEditor(
        host,
        initial,
        people,
        (op, args) => api(op, args, active),
        () => save(),
        action,
      );
  const editor = richEditor(
    host,
    draft || { body: composeContext?.body || "" },
    () => save(),
  );
  const data = () => ({
    slot,
    account: q("sender")?.value || sender,
    ...values(),
    personal: q("mode")?.value === "personal",
    parent: parentId,
    subject:
      q("subject")?.value ??
      draft?.subject ??
      (/^re:/i.test(parent.subject) ? parent.subject : "Re: " + parent.subject),
    ...editor.value(),
    signature,
    forwarded: storedFiles.map((f) => f.id),
    storedFiles: [...storedFiles],
    fileStamps: Object.fromEntries(stamps),
    files: [...files],
  });
  async function inspectFiles(paths) {
    if (!paths.length) return;
    const observed = await api("file_stamps", { paths }, active);
    paths.forEach((path, i) => {
      if (!files.includes(path)) return;
      const stamp = observed[i];
      issues.delete(path);
      if (!stamp) issues.set(path, "Файл недоступен");
      else if (stamps.has(path) && stamps.get(path) !== stamp)
        issues.set(path, "Файл изменился");
      else stamps.set(path, stamp);
    });
  }
  async function addFiles(paths) {
    files = [...new Set([...files, ...paths])];
    save();
    await inspectFiles(paths);
    if (!editorRoot.isConnected) return;
    renderFiles();
    save();
  }
  const renderFiles = () => {
    q("files").replaceChildren();
    storedFiles.forEach((f, i) => {
      const button = document.createElement("button");
      button.textContent = f.name + " ×";
      button.onclick = () => {
        storedFiles.splice(i, 1);
        renderFiles();
        save();
      };
      q("files").append(button);
    });
    files.forEach((f, i) => {
      let b = document.createElement("button");
      if (issues.has(f)) {
        const warning = document.createElement("span");
        warning.textContent = issues.get(f) + " · " + basename(f) + " ";
        const replace = document.createElement("button");
        replace.textContent = "Выбрать замену";
        replace.onclick = action(async () => {
          const picked = await api("pick");
          if (!picked.length) return;
          files = files.filter((path) => path !== f);
          stamps.delete(f);
          issues.delete(f);
          await addFiles(picked);
        });
        q("files").append(warning, replace);
      }
      b.textContent = basename(f) + " ×";
      b.onclick = () => {
        files.splice(i, 1);
        stamps.delete(f);
        issues.delete(f);
        renderFiles();
        save();
      };
      q("files").append(b);
    });
  };
  const hasContent = (draft) =>
    draft.files.length > 0 ||
    draft.forwarded.length > 0 ||
    draft.body.replace(/[\s\u200b]/gu, "").length > 0;
  let previewVersion = 0;
  const preview = async () => {
    let version = ++previewVersion;
    const d = data();
    q("send").disabled = true;
    try {
      let targets = await api("recipient_plan", d, active);
      let to = targets.map((r) => r.email);
      if (version !== previewVersion) return;
      q("preview").textContent = parent
        ? (all ? "Ответ всем" : "Ответ") + " · " + to.join(", ")
        : (accounts.find((a) => a.id === d.account)?.address || "") +
          " → " +
          to.length +
          " · " +
          (d.personal ? "Каждый видит только свой адрес" : visibility(targets));
      q("send").disabled = !to.length || !hasContent(d) || issues.size > 0;
    } catch {
      if (version === previewVersion)
        q("preview").textContent = "Укажите адреса получателей через запятую";
    }
  };
  const save = () => {
    const d = data();
    draftWrites = draftWrites
      .then(() => api("save_draft", { slot, data: d }, active))
      .catch(err);
    preview();
  };
  host.oninput = (e) => {
    if (!q("snippets").contains(e.target)) save();
  };
  q("templates").onclick = action(() => {
    const insert = editor.insertSnippet();
    return snippetMenu(
      q("snippets"),
      (op, args) => api(op, args, active),
      action,
      insert,
    );
  });
  if (!parent)
    q("sender").onchange = () => {
      signature =
        accounts.find((a) => a.id === q("sender").value)?.signature || "";
      q("signature").textContent = signature;
      save();
    };
  q("signature").textContent = signature;
  const editorRoot = q("body");
  const baselineCount = stamps.size;
  await inspectFiles([...files]);
  if (!editorRoot.isConnected || active !== pid) return;
  if (stamps.size !== baselineCount) save();
  renderFiles();
  preview();
  if (
    retarget ||
    (!draft && (initialTo || composeContext?.body || composeContext?.subject))
  )
    save();
  host.ondropfiles = action(addFiles);
  q("attach").onclick = action(async () => addFiles(await api("pick")));
  let submitting = false;
  q("send").onclick = action(async () => {
    if (submitting) return;
    submitting = true;
    try {
      await draftWrites;
      const d = data();
      await inspectFiles([...files]);
      if (issues.size) {
        renderFiles();
        preview();
        return;
      }
      if (!hasContent(d)) return;
      if (
        !d.files.length &&
        !d.forwarded.length &&
        /во вложении|прикладываю|прикрепляю|attached|see attachment/i.test(
          d.body,
        ) &&
        (await ask(
          "В тексте упоминается вложение. Добавить файл перед отправкой?",
        ))
      ) {
        await addFiles(await api("pick"));
        return;
      }
      let targets = await api("recipient_plan", d, active);
      let to = targets.map((r) => r.email);
      const from = accounts.find((a) => a.id === d.account)?.address;
      if (
        !(await ask(
          `От: ${from}\nПолучателей: ${to.length}\n${d.personal ? "У каждого в поле To только собственный адрес" : visibility(targets)}\nОтправить?`,
        ))
      )
        return;
      const batch = await api("enqueue", d, active);
      if (composeContext) {
        await window.__TAURI__.event
          .emitTo("main", "compose-sent", { batch, profile: active })
          .catch(err);
        await window.__TAURI__.window.getCurrentWindow().destroy();
        return;
      }
      undo("Отменить отправку", active, async () => {
        await cancelSend(batch, active);
      });
      host.oninput = null;
      if (parent) host.replaceChildren();
      else shut();
      status("Письмо в очереди");
      await list();
    } finally {
      submitting = false;
    }
  });
}
function visibility(targets) {
  const visible = targets.filter((r) => r.field !== "bcc").map((r) => r.email),
    hidden = targets.filter((r) => r.field === "bcc").map((r) => r.email);
  return (
    "Всем видны: " +
    (visible.join(", ") || "—") +
    (hidden.length ? " · Скрытая копия: " + hidden.join(", ") : "")
  );
}
async function compose(to = "", slot = "", subject = "", body = "") {
  if (!pid) return;
  if (!accounts.length) {
    accountDialog(null);
    return;
  }
  await window.__TAURI__.core.invoke("compose_window", {
    profile: pid,
    slot: slot || "new:" + crypto.randomUUID(),
    to,
    subject,
    body,
  });
}
async function followLink(url) {
  if (/^mailto:/i.test(url)) {
    const d = mailtoDraft(url);
    await compose(d.to, "", d.subject, d.body);
  } else await api("open_url", { url });
}
async function refreshMail() {
  await refreshFolders();
  await list();
}
async function cancelSend(batch, profile) {
  const draft = await api("cancel_send", { batch }, profile);
  if (draft.parent && draft.slot.startsWith("reply:"))
    await openThread(draft.slot.slice(6), draft.parent);
  else await compose("", draft.slot);
}
function undo(label, profile, fn) {
  const b = document.createElement("button");
  b.textContent = label;
  b.onclick = action(async () => {
    if (pid !== profile) return;
    b.remove();
    await fn();
  });
  $("undo-actions").append(b);
  setTimeout(() => b.remove(), 10000);
}
async function bulk(key, ids, profile) {
  const run = async (folder = "", until = 0) => {
    if (pid !== profile) return;
    const move = ["archive", "trash", "move"].includes(key);
    const batch = await api(
      "bulk",
      { action: move ? "move" : key, ids, folder, until },
      profile,
    );
    if (move) {
      undo("Отменить перемещение", profile, async () => {
        await api("undo_move", { batch }, profile);
        await refreshMail();
      });
    }
    await refreshMail();
  };
  if (key === "move" || key === "snooze") {
    const tomorrow = new Date();
    tomorrow.setDate(tomorrow.getDate() + 1);
    tomorrow.setHours(9, 0, 0, 0);
    const local = new Date(
      tomorrow.getTime() - tomorrow.getTimezoneOffset() * 60000,
    )
      .toISOString()
      .slice(0, 16);
    modal(
      key === "move" ? "Переместить письма" : "Отложить переписку",
      key === "move"
        ? '<label>Папка<input id="bulk-dest" list="mail-folders"></label><datalist id="mail-folders">' +
            options(
              [...new Set(folders.map((f) => f.name))].map((n) => [n, n]),
              "",
            ) +
            '</datalist><button id="apply-bulk">Переместить</button>'
        : '<label>Вернуть в список<input type="datetime-local" id="bulk-date" value="' +
            local +
            '"></label><button id="apply-bulk">Отложить</button>',
    );
    click("apply-bulk", async () => {
      await run(
        $("bulk-dest")?.value,
        Math.floor(new Date($("bulk-date")?.value).getTime() / 1000),
      );
      shut();
    });
  } else
    await run(key === "archive" ? "Archive" : key === "trash" ? "Trash" : "");
}
async function refreshFolders() {
  const active = pid;
  const [data, display] = await Promise.all([
    api("folders"),
    api("mailbox_display"),
  ]);
  if (active === pid) {
    folders = data;
    mailboxDisplay = display;
    renderNav();
  }
}
async function synchronize(active) {
  if (!active || syncProfile === active) return;
  syncProfile = active;
  const batch = [...accounts];
  const errors = [];
  if (!folders.length && batch.length)
    $("threads").innerHTML = empty("Первичное получение почты…");
  for (const a of batch) {
    if (pid !== active) break;
    liveSync.add(active + "/" + a.id);
    renderNav();
    status("Получение почты · " + a.address);
    try {
      const count = await api("sync", { account: a.id }, active);
      if (count > 0 && prefs.notify && pid === active)
        await window.__TAURI__.core.invoke("notify_mail", {
          account: a.id,
          profile: active,
          count,
        });
    } catch (e) {
      errors.push(a.address + " · " + String(e));
    } finally {
      liveSync.delete(active + "/" + a.id);
    }
  }
  if (syncProfile === active) syncProfile = "";
  if (pid !== active) return;
  await refreshFolders();
  await list();
  status(errors.join(" / ") || "Синхронизация завершена");
}
async function background(active, check = true) {
  try {
    await api("deliver", {}, active);
    if (active === pid && check) await synchronize(active);
  } catch (e) {
    err(e);
  }
}
const book = addressBook({
  api,
  modal,
  click,
  action,
  shut,
  compose,
  openThread,
  date,
  onChange: (rows) => {
    people = rows;
    renderNav();
  },
  profile: () => pid,
});
const states = {
  held: "Можно отменить",
  pending: "В очереди",
  sending: "Отправляется",
  sent: "Принято сервером",
  failed: "Ошибка",
  unknown: "Статус неизвестен",
};
async function queue() {
  const jobs = await api("queue");
  modal(
    "Очередь отправки",
    `<div class="hint">Принято сервером: ${jobs.filter((j) => j.state === "sent").length} из ${jobs.length}</div><div id="jobs"></div><button id="reload">Обновить</button>`,
  );
  let batch = "";
  for (const j of jobs) {
    if (j.batch !== batch) {
      batch = j.batch;
      const group = jobs.filter((x) => x.batch === batch);
      let heading = document.createElement("h3");
      heading.textContent =
        date(j.created) +
        " · Принято " +
        group.filter((x) => x.state === "sent").length +
        " / " +
        group.length;
      $("jobs").append(heading);
      if (j.state === "held") {
        const cancel = document.createElement("button");
        cancel.textContent = "Отменить отправку";
        cancel.onclick = action(async () => {
          shut();
          await cancelSend(j.batch, pid);
        });
        $("jobs").append(cancel);
      }
    }
    let row = document.createElement("div");
    row.className = "entry";
    row.innerHTML = `<strong>${h(j.recipients)}</strong><p>${h(states[j.state])} · ${h(accounts.find((a) => a.id === j.account)?.address)}</p><small>${h(j.error)}</small>`;
    if (j.copy_job) {
      const info = document.createElement("p");
      info.textContent = j.copy_folder
        ? "Копия на сервере · " + j.copy_folder
        : j.copy_error
          ? "Не удалось сохранить копию на сервере · " + j.copy_error
          : "Сохранение копии на сервере…";
      row.append(info);
      if (j.copy_error) {
        const retry = document.createElement("button");
        retry.textContent = "Повторить сохранение копии";
        retry.onclick = action(async () => {
          await api("retry_copy", { id: j.id });
          await queue();
        });
        row.append(retry);
      }
    }
    if (["failed", "unknown"].includes(j.state)) {
      let b = document.createElement("button");
      b.textContent = "Повторить";
      b.onclick = action(async () => {
        if (
          j.state === "unknown" &&
          !(await ask("Сервер мог принять письмо. Отправить повторно?"))
        )
          return;
        await api("retry", { id: j.id });
        await api("deliver");
        await queue();
      });
      row.append(b);
    }
    $("jobs").append(row);
  }
  click("reload", queue);
  if (jobs.some((j) => ["held", "pending", "sending"].includes(j.state))) {
    const panel = $("jobs");
    setTimeout(() => {
      if (panel.isConnected && $("dialog").open) queue().catch(err);
    }, 1200);
  }
}
click("profiles", profileDialog);
click("add", () => accountDialog(null));
click("theme", () => {
  document.documentElement.classList.toggle("dark");
  localStorage.setItem("theme", document.documentElement.className);
});
click("sync", () => synchronize(pid));
click("compose", () => compose());
click("contacts", () => book.open());
click("queue", queue);
click("close", shut);
$("dialog").addEventListener("close", () => {
  if (quickView === "drafts") list().catch(err);
});
click("navtoggle", () => document.body.classList.toggle("navopen"));
click("filter", () => {
  $("filters").hidden = !$("filters").hidden;
});
$("profile").onchange = action(switchProfile);
let timer;
$("search").oninput = () => {
  if ($("search").value && (selection.account || selection.folder)) {
    selection = { account: "", folder: "" };
    $("fa").value = "";
    $("ff").value = "";
    renderNav();
  }
  clearTimeout(timer);
  timer = setTimeout(action(list), 120);
};
function openCommands() {
  if (composeContext || document.querySelector("dialog[open]")) return;
  const previous = document.activeElement;
  const commands = [
    ...mailList.commands(),
    { name: "Написать письмо", run: () => compose() },
    {
      name: "Поиск писем",
      run: () => {
        $("search").focus();
      },
    },
    { name: "Настройки", run: () => $("settings").click() },
  ];
  modal(
    "Быстрые действия",
    '<input id="command-search" aria-label="Найти действие" placeholder="Найти действие"><div id="command-list"></div>',
  );
  const dialog = $("dialog"),
    input = $("command-search"),
    list = $("command-list");
  dialog.addEventListener(
    "close",
    () => {
      if (
        !dialog.open &&
        document.activeElement === document.body &&
        previous instanceof HTMLElement &&
        previous.isConnected
      )
        previous.focus();
    },
    { once: true },
  );
  function render() {
    list.replaceChildren();
    for (const cmd of commands.filter((cmd) =>
      cmd.name.toLowerCase().includes(input.value.trim().toLowerCase()),
    )) {
      const b = document.createElement("button");
      b.textContent = cmd.name;
      b.onclick = action(async () => {
        shut();
        if (previous instanceof HTMLElement) previous.focus();
        await cmd.run();
      });
      list.append(b);
    }
    if (!list.children.length) list.textContent = "Действий не найдено";
  }
  input.oninput = render;
  const controller = new AbortController();
  dialog.addEventListener("close", () => controller.abort(), { once: true });
  $("content").addEventListener(
    "keydown",
    function navigate(e) {
      if (!input.isConnected) return;
      const buttons = Array.from(list.querySelectorAll("button")),
        idx = buttons.indexOf(document.activeElement);
      if (e.key === "Enter" && e.target === input) {
        e.preventDefault();
        buttons[0]?.click();
      }
      if (buttons.length && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
        e.preventDefault();
        buttons[
          idx < 0
            ? e.key === "ArrowDown"
              ? 0
              : buttons.length - 1
            : (idx + (e.key === "ArrowDown" ? 1 : -1) + buttons.length) %
              buttons.length
        ].focus();
      }
    },
    { signal: controller.signal },
  );
  render();
  input.focus();
}
document.addEventListener("keydown", (e) => {
  if (!(e.metaKey || e.ctrlKey)) return;
  const key = e.key.toLowerCase();
  if (key === "k" && !composeContext) {
    e.preventDefault();
    openCommands();
    return;
  }
  if (key === "enter") {
    e.preventDefault();
    const host = composeContext
      ? $("composeform")
      : $("dialog").open
        ? $("content")
        : $("quick");
    host?.querySelector('[data-field="send"]')?.click();
  } else if (key === "f" && !$("dialog").open) {
    e.preventDefault();
    document.body.classList.remove("reading");
    $("search").focus();
  } else if (key === "n" && !$("dialog").open) {
    e.preventDefault();
    action(() => compose())(e);
  } else if (key === "r" && !$("dialog").open) {
    e.preventDefault();
    $("quick")?.querySelector('[data-field="body"]')?.focus();
  }
});
document.documentElement.className = localStorage.getItem("theme") || "";
if (composeContext) {
  document.body.classList.add("compose-window");
  pid = composeContext.profile;
  Promise.all([api("accounts"), api("contacts")])
    .then(async ([boxes, contacts]) => {
      accounts = boxes;
      people = contacts;
      await composer(
        $("composeform"),
        null,
        false,
        false,
        composeContext.to,
        composeContext.slot,
      );
    })
    .catch(err);
} else
  loadProfiles(localStorage.getItem("profile")).then(checkChanges).catch(err);
window.__TAURI__.webview
  ?.getCurrentWebview()
  .onDragDropEvent(({ payload }) => {
    const host = composeContext ? $("composeform") : $("quick");
    if (!host?.querySelector(".composer")) return;
    const rect = host.getBoundingClientRect();
    const pos = payload.position,
      scale = window.devicePixelRatio || 1;
    const inside =
      !!composeContext ||
      (pos &&
        pos.x / scale >= rect.left &&
        pos.x / scale <= rect.right &&
        pos.y / scale >= rect.top &&
        pos.y / scale <= rect.bottom);
    host.classList.toggle(
      "drop-target",
      inside && ["enter", "over"].includes(payload.type),
    );
    if (payload.type === "drop" && inside) host.ondropfiles?.(payload.paths);
  })
  .catch(err);
if (!composeContext)
  window.__TAURI__.event
    .listen(
      "compose-sent",
      action(async ({ payload }) => {
        if (pid === payload.profile) {
          undo("Отменить отправку", pid, () =>
            cancelSend(payload.batch, payload.profile),
          );
          status("Письмо в очереди");
          await list();
        }
      }),
    )
    .catch(err);
function schedule() {
  nextPoll = 0;
  backgroundSync.clear();
  return window.__TAURI__.core
    .invoke("set_schedule", {
      profile: pid,
      minutes: prefs.minutes,
      instant: prefs.instant,
      notify: prefs.notify,
    })
    .catch(err);
}
if (!composeContext)
  window.__TAURI__.event
    .listen(
      "open-mail",
      action(async ({ payload }) => {
        if (pid !== payload.profile) await loadProfiles(payload.profile);
        if (pid === payload.profile) {
          shut();
          await openThread(payload.thread, payload.id);
        }
      }),
    )
    .catch(err);
if (!composeContext)
  window.__TAURI__.event
    .listen(
      "scheduled-mail",
      action(async ({ payload }) => {
        if (pid !== payload.profile) return;
        await refreshFolders();
        await list();
        if (pid === payload.profile)
          status(payload.error || "Синхронизация завершена");
      }),
    )
    .catch(err);
$("sort").value = prefs.sort;
$("sort").onchange = action(async () => {
  prefs.sort = $("sort").value;
  persist();
  await list();
});
const updates = updateChecks({
  hours: () => prefs.updateHours,
  storage: localStorage,
  fetchVersion: () =>
    window.__TAURI__.core.invoke("update", { installVersion: null }),
  announce: async (version) => {
    const button = $("update-alert");
    button.hidden = !version;
    if (!version) return;
    button.textContent = "Доступно обновление · " + version;
    if (localStorage.getItem("update-notified") !== version) {
      await window.__TAURI__.core.invoke("notify_update", { version });
      localStorage.setItem("update-notified", version);
    }
  },
});
if (!composeContext) updates.schedule();
click("update-alert", () => $("settings").click());
click(
  "settings",
  settingsMenu({
    modal,
    click,
    api,
    schedule,
    showChanges,
    restoreBackup,
    hasProfile: () => !!pid,
    showDiskUsage: async function view(back) {
      modal(
        "Место на диске",
        '<button id="disk-back">← Настройки</button><div id="disk-results" role="status"></div>',
      );
      $("dialog").classList.add("wide");
      click("disk-back", back);
      const host = $("disk-results");
      if (!pid) {
        host.textContent = "Откройте профиль для просмотра статистики.";
        return;
      }
      const profile = pid;
      host.textContent = "Подсчёт данных профиля…";
      try {
        const report = await api("disk_usage", {}, profile);
        if (!host.isConnected || pid !== profile) return;
        host.innerHTML = diskReport(report);
        click("clean-opened", async () => {
          if (await ask("Удалить неизменённые открытые копии старше 7 дней?")) {
            await api("clean_opened", {}, profile);
            await view(back);
          }
        });
        for (const button of host.querySelectorAll("[data-message]")) {
          button.onclick = action(async () => {
            const message = report.largest[Number(button.dataset.message)];
            shut();
            await openThread(message.thread, message.id);
          });
        }
      } catch (error) {
        if (host.isConnected)
          host.textContent = "Не удалось подсчитать размер: " + error;
      }
    },
    updates,
    rules: rulesPanel({ api, modal, click, action }),
    flush: () => draftWrites,
  }),
);
const appWindow = window.__TAURI__.window.getCurrentWindow();
api("about")
  .then((info) => {
    if (info.windows && !composeContext) {
      $("titlebar").hidden = false;
      document.body.classList.add("windows");
      click("minimize", () => appWindow.minimize());
      click("maximize", () => appWindow.toggleMaximize());
      click("winclose", () => appWindow.close());
    }
  })
  .catch(err);
appWindow
  .onCloseRequested(async (event) => {
    event.preventDefault();
    await draftWrites;
    if (composeContext)
      await window.__TAURI__.event
        .emitTo("main", "composer-closed", { profile: pid })
        .catch(err);
    await window.__TAURI__.core.invoke("finish_close", {});
  })
  .catch(err);

if (!composeContext)
  window.__TAURI__.event
    .listen(
      "composer-closed",
      action(async ({ payload }) => {
        if (pid === payload.profile && quickView === "drafts") await list();
      }),
    )
    .catch(err);
function showChanges(info, previous = "") {
  const dlg = $("changes");
  const entries = info.changelog.split(/^## /m).slice(1);
  const end = entries.findIndex(
    (entry) => entry.split("\n")[0].trim() === previous,
  );
  const sections = (end < 0 ? entries : entries.slice(0, end)).map((entry) => {
    const [version, ...lines] = entry.trim().split("\n");
    const section = document.createElement("section");
    const heading = document.createElement("h3");
    heading.textContent = "Версия " + version;
    const list = document.createElement("ul");
    for (const line of lines.filter((line) => line.startsWith("- "))) {
      const item = document.createElement("li");
      item.textContent = line.slice(2);
      list.append(item);
    }
    section.append(heading, list);
    return section;
  });
  $("changes-text").replaceChildren(...sections);
  dlg.showModal();
}
async function checkChanges() {
  const info = await api("about");
  if (!info.version || !info.changelog) return;
  const previous = localStorage.getItem("seen-release");
  if (previous !== info.version && (previous || pid))
    showChanges(info, previous || "0.4.1");
  localStorage.setItem("seen-release", info.version);
}
$("changes-close").onclick = () => $("changes").close();
