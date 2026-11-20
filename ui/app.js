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
import { escapeHtml as h, options, splitQuote, basename } from "./helpers.js";
/** @returns {any} */
const $ = (id) => document.getElementById(id);
const api = (op, args = {}, profile = pid) =>
  window.__TAURI__.core.invoke("request", { op, args, profile });
let pid = "",
  accounts = [],
  folders = [],
  people = [],
  thread = "",
  listVersion = 0,
  readerVersion = 0;
let selection = { account: "", folder: "" },
  syncProfile = "";
let quickView = "";
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
    '<label>Название нового профиля<input id="pname" placeholder="Работа" autofocus></label><div class="actions"><button id="pdefault" class="accent">Создать в стандартной папке</button><button id="pcustom">Выбрать папку…</button></div><hr><button id="popen">Открыть существующий профиль…</button>',
  );
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
  schedule();
  localStorage.setItem("profile", pid);
  thread = "";
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
  const data = await Promise.all(
    ["accounts", "folders", "contacts"].map((op) => api(op, {}, active)),
  );
  if (pid !== active) return;
  [accounts, folders, people] = data;
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
  unread: "● Непрочитанные",
  starred: "★ Со звездой",
  files: "▧ С вложениями",
  drafts: "✎ Черновики",
};
function renderNav() {
  let html = '<button data-index="-1">▤　Вся почта</button>';
  for (const [id, label] of Object.entries(views)) {
    html += `<button data-view="${id}" class="${quickView === id ? "active" : ""}">${h(label)}</button>`;
  }
  accounts.forEach((a, i) => {
    html += `<div class="accountname"><button data-account="${i}" title="${h(a.address)}">▣　${h(a.name || a.address)}</button><button data-edit="${i}" title="Настройки ящика">⚙</button></div>`;
    folders.forEach((f, j) => {
      if (f.account === a.id)
        html += `<button class="folder" data-index="${j}">▱　${h(f.name)}</button>`;
    });
  });
  $("folders").innerHTML = html;
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
  el.innerHTML = `<label>Ящик<select id="fa">${options([["", "Все ящики"], ...accounts.map((a) => [a.id, a.address])], selection.account)}</select></label><label>Папка<select id="ff">${options([["", "Все папки"], ...Array.from(new Set(folders.map((f) => f.name))).map((f) => [f, f])], selection.folder)}</select></label><label>С даты<input type="date" id="from"></label><label>По дату<input type="date" id="until"></label><label class="check"><input type="checkbox" id="hasfiles"> С вложениями</label>`;
  el.onchange = action(list);
}
let activeMessage = null;
const mailList = threadList({
  api,
  action,
  date,
  open: openThread,
  refresh: list,
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
async function openThread(key, hit) {
  await draftWrites;
  const token = ++readerVersion;
  thread = key;
  activeMessage = hit;
  const data = await api("thread", { thread: key, hit });
  if (token !== readerVersion) return;
  const last = data.at(-1);
  if (!last) return false;
  document.body.classList.add("reading");
  $("reader").innerHTML =
    `<div class="readerhead"><button id="back">← Список</button><h1>${h(last.subject || "Без темы")}</h1></div><div id="timeline" class="timeline"></div><div id="quick"></div>`;
  click("back", () => document.body.classList.remove("reading"));
  for (const m of data) {
    const files = await api("attachments", { id: m.id });
    if (token !== readerVersion) return;
    const body = splitQuote(m.body)
      .map(({ quoted, text }) => {
        const block = "<pre>" + h(text) + "</pre>";
        return quoted
          ? "<details><summary>Цитата</summary>" + block + "</details>"
          : block;
      })
      .join("");
    let art = document.createElement("article");
    art.className =
      "bubble" +
      (m.outgoing ? " out" : "") +
      (m.id === hit && $("search").value ? " hit" : "");
    art.id = "msg" + m.id;
    art.innerHTML = `<strong>${h(m.sender)}</strong><div class="meta">Кому: ${h(m.recipients)} · ${h(date(m.stamp))}</div><div class="plain-body">${body}</div><div class="parts"></div><div class="actions"></div>`;
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
      btn(actions, "HTML", () => htmlView(art, m, files, false));
      btn(actions, "Загрузить внешние изображения", () =>
        htmlView(art, m, files, true),
      );
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
    $("timeline").append(art);
  }
  await composer($("quick"), last, false, false);
  if (token !== readerVersion) return;
  const target = $("msg" + (hit || last.id));
  target?.scrollIntoView({ block: "nearest" });
  await list();
  return true;
}
async function htmlView(art, m, files, remote) {
  const template = document.createElement("template");
  template.innerHTML = m.html;
  for (const img of template.content.querySelectorAll("img")) {
    const src = img.getAttribute("src") || "";
    img.removeAttribute("src");
    if (src.startsWith("cid:")) {
      const f = files.find((f) => f.cid === src.slice(4));
      if (f) img.src = await api("inline", { id: f.id });
    } else if (remote && /^https:\/\//i.test(src)) img.src = src;
  }
  let frame = art.querySelector("iframe") || document.createElement("iframe");
  frame.setAttribute("sandbox", "");
  frame.title = "Содержимое письма";
  frame.referrerPolicy = "no-referrer";
  frame.srcdoc =
    `<meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data: ${remote ? "https:" : ""}; style-src 'unsafe-inline'"><meta name="referrer" content="no-referrer"><style>body{font:15px system-ui;overflow-wrap:anywhere}img{max-width:100%}</style>` +
    template.innerHTML;
  art.querySelector(".plain-body").hidden = true;
  art.append(frame);
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
    `<div class="grid">${input("aname", "Имя отправителя", a.name)}${input("address", "Email", a.address, "email")}</div><div class="actions"><button id="discover">Найти настройки по email</button><select id="applypreset" aria-label="Применить шаблон">${options([["", "Шаблон подключения…"], ...prefs.presets.map((p) => [p.id, p.name])], "")}</select></div>${server("in", "Входящая почта", a.incoming)}<label class="check"><input id="keep" type="checkbox" ${a.keep ? "checked" : ""}> POP3 · Оставлять письма на сервере</label>${server("out", "Исходящая почта", a.outgoing)}<label class="check"><input id="remember" type="checkbox" ${localStorage.getItem("remember:" + pid + ":" + a.id) !== "false" ? "checked" : ""}> Запомнить пароли на этом компьютере</label><label>Подпись<textarea id="signature">${h(a.signature)}</textarea></label><button id="saveaccount" class="accent">Сохранить ящик</button>`,
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
      remember: $("remember").checked,
      in: $("inpass").value,
      out: $("outpass").value,
    };
  };
  for (const side of ["in", "out"])
    click("test" + side, async () => {
      $("error").textContent = "Подключение…";
      await api("test", { ...data(), side });
      $("error").textContent = "Соединение установлено";
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
  let files = draft?.files || [],
    parentId = retarget ? parent?.id : draft?.parent || parent?.id || null;
  const q = (k) => host.querySelector('[data-field="' + k + '"]');
  host.innerHTML = `<div class="composer"><div class="line"><label>Отправитель<select data-field="sender">${accountOptions(sender)}</select></label>${
    !parent
      ? '<label>Режим<select data-field="mode">' +
        options(
          [
            ["group", "Общее письмо"],
            ["personal", "Персональная рассылка"],
          ],
          draft?.personal ? "personal" : "group",
        ) +
        "</select></label>"
      : ""
  }</div>${recipientFields()}<div data-field="suggest" class="actions"></div><label ${parent ? "hidden" : ""}>Тема<input data-field="subject" value="${h(draft?.subject || (parent ? (/^re:/i.test(parent.subject) ? parent.subject : "Re: " + parent.subject) : ""))}"></label><textarea aria-label="Текст письма" data-field="body" placeholder="${parent ? "Ответить в переписке…" : "Текст письма…"}">${h(draft?.body || "")}</textarea><button data-field="templates">Шаблоны ответов</button><div data-field="snippets" hidden></div><div class="signature" data-field="signature"></div><div data-field="files"></div><div class="hint" data-field="preview"></div><div class="actions"><button data-field="attach">＋ Вложение</button><button class="accent" data-field="send">Отправить ↗</button></div></div>`;
  const values = recipientEditor(
    host,
    { to, cc: draft?.cc || "", bcc: draft?.bcc || "" },
    people,
    (op, args) => api(op, args, active),
    () => save(),
    action,
  );
  const data = () => ({
    slot,
    account: q("sender").value,
    ...values(),
    personal: q("mode")?.value === "personal",
    parent: parentId,
    subject: q("subject").value,
    body: q("body").value,
    signature,
    files: [...files],
  });
  const renderFiles = () => {
    q("files").replaceChildren();
    files.forEach((f, i) => {
      let b = document.createElement("button");
      b.textContent = basename(f) + " ×";
      b.onclick = () => {
        files.splice(i, 1);
        renderFiles();
        save();
      };
      q("files").append(b);
    });
  };
  let previewVersion = 0;
  const preview = async () => {
    let version = ++previewVersion;
    const d = data();
    q("send").disabled = true;
    try {
      let targets = await api("recipient_plan", d, active);
      let to = targets.map((r) => r.email);
      if (version !== previewVersion) return;
      q("preview").textContent =
        (accounts.find((a) => a.id === d.account)?.address || "") +
        " → " +
        to.length +
        " · " +
        (d.personal ? "Каждый видит только свой адрес" : visibility(targets));
      q("send").disabled = false;
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
    const body = q("body"),
      start = body.selectionStart,
      end = body.selectionEnd;
    return snippetMenu(
      q("snippets"),
      (op, args) => api(op, args, active),
      action,
      (text) => {
        body.setRangeText(text, start, end, "end");
        body.focus();
        save();
      },
    );
  });
  q("sender").onchange = () => {
    signature =
      accounts.find((a) => a.id === q("sender").value)?.signature || "";
    q("signature").textContent = signature;
    save();
  };
  q("signature").textContent = signature;
  renderFiles();
  preview();
  if (retarget || initialTo) save();
  q("attach").onclick = action(async () => {
    files.push(...(await api("pick")));
    renderFiles();
    save();
  });
  let submitting = false;
  q("send").onclick = action(async () => {
    if (submitting) return;
    submitting = true;
    try {
      await draftWrites;
      const d = data();
      let targets = await api("recipient_plan", d, active);
      let to = targets.map((r) => r.email);
      const from = accounts.find((a) => a.id === d.account)?.address;
      if (
        !(await ask(
          `От: ${from}\nПолучателей: ${to.length}\n${d.personal ? "У каждого в поле To только собственный адрес" : visibility(targets)}\nОтправить?`,
        ))
      )
        return;
      await api("enqueue", d, active);
      host.oninput = null;
      if (parent) host.replaceChildren();
      else shut();
      status("Письмо в очереди");
      await api("deliver", {}, active);
      if (pid === active) {
        await refreshQueueStatus();
        await refreshFolders();
        await list();
        if (parent && thread === parent.thread) await openThread(thread, null);
      }
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
async function compose(to = "", slot = "") {
  if (!pid) return;
  modal("Новое письмо", '<div id="composeform"></div>');
  await composer(
    $("composeform"),
    null,
    false,
    false,
    to,
    slot || "new:" + crypto.randomUUID(),
  );
}
async function refreshFolders() {
  const active = pid;
  const data = await api("folders");
  if (active === pid) {
    folders = data;
    renderNav();
  }
}
async function refreshQueueStatus() {
  const jobs = await api("queue");
  const failures = jobs.filter(
    (j) => j.state === "failed" || j.state === "unknown",
  );
  status(
    failures.length
      ? "Требуют внимания в очереди: " + failures.length
      : "Очередь обработана",
  );
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
    status("Получение почты · " + a.address);
    try {
      const count = await api("sync", { account: a.id }, active);
      if (count > 0 && prefs.notify && pid === active)
        await window.__TAURI__.core.invoke("notify_mail", {
          account: a.address,
          count,
        });
    } catch (e) {
      errors.push(a.address + " · " + String(e));
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
    }
    let row = document.createElement("div");
    row.className = "entry";
    row.innerHTML = `<strong>${h(j.recipients)}</strong><p>${h(states[j.state])} · ${h(accounts.find((a) => a.id === j.account)?.address)}</p><small>${h(j.error)}</small>`;
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
  if (jobs.some((j) => ["pending", "sending"].includes(j.state))) {
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
document.addEventListener("keydown", (e) => {
  if (!(e.metaKey || e.ctrlKey)) return;
  const key = e.key.toLowerCase();
  if (key === "enter") {
    e.preventDefault();
    const host = $("dialog").open ? $("content") : $("quick");
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
    $("quick")?.querySelector("textarea")?.focus();
  }
});
document.documentElement.className = localStorage.getItem("theme") || "";
loadProfiles(localStorage.getItem("profile")).catch(err);
function schedule() {
  return window.__TAURI__.core
    .invoke("set_schedule", {
      profile: pid,
      minutes: prefs.minutes,
      notify: prefs.notify,
    })
    .catch(err);
}
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
click(
  "settings",
  settingsMenu({ modal, click, api, schedule, flush: () => draftWrites }),
);
const appWindow = window.__TAURI__.window.getCurrentWindow();
api("about")
  .then((info) => {
    if (info.windows) {
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
    await window.__TAURI__.core.invoke("finish_close", {});
  })
  .catch(err);
