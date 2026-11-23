import { escapeHtml as h, options } from "./helpers.js";

export const prefs = {
  startup: true,
  minutes: 5,
  updateHours: 24,
  instant: true,
  notify: true,
  sort: "newest",
  presets: [],
  ...JSON.parse(localStorage.getItem("mail-preferences") || "{}"),
};
export function persist() {
  localStorage.setItem("mail-preferences", JSON.stringify(prefs));
}

export function serverFields(k, server) {
  const incoming = k === "in";
  return `<div class="grid"><label>Протокол<select id="${k}proto">${options(
    (incoming ? ["imap", "pop3"] : ["smtp"]).map((p) => [p, p.toUpperCase()]),
    server.proto,
  )}</select></label>
    <label>Шифрование<select id="${k}tls">${options(
      [
        ["tls", "TLS"],
        ["starttls", "STARTTLS"],
      ],
      server.tls,
    )}</select></label>
    <label>Сервер<input id="${k}host" value="${h(server.host)}"></label>
    <label>Порт<input id="${k}port" type="number" min="1" max="65535" value="${h(server.port)}"></label></div>`;
}
const get = (id) =>
  /** @type {HTMLInputElement} */ (document.getElementById(id));
export function bindPorts() {
  for (const k of ["in", "out"]) {
    const change = () => {
      get(k + "port").value = String(
        { imap: [993, 143], pop3: [995, 110], smtp: [465, 587] }[
          get(k + "proto").value
        ][get(k + "tls").value === "tls" ? 0 : 1],
      );
    };
    get(k + "tls").onchange = change;
    get(k + "proto").onchange = change;
  }
}
export function endpoint(k) {
  return {
    proto: get(k + "proto").value,
    host: get(k + "host").value.trim(),
    port: Number(get(k + "port").value),
    tls: get(k + "tls").value,
  };
}
export function applyConfig(config, address) {
  for (const [key, server] of [
    ["in", config.incoming],
    ["out", config.outgoing],
  ]) {
    for (const field of ["proto", "host", "port", "tls"])
      get(key + field).value = server[field];
  }
  const expand = (login) =>
    (login || "%EMAILADDRESS%")
      .replaceAll("%EMAILADDRESS%", address)
      .replaceAll("%EMAILLOCALPART%", address.split("@")[0]);
  get("inlogin").value = expand(config.login);
  get("outlogin").value = expand(config.smtp_login);
}
export function settingsMenu({
  modal,
  click,
  api,
  schedule,
  flush,
  rules,
  showChanges,
  restoreBackup,
  hasProfile = () => true,
  showDiskUsage,
  updates,
}) {
  async function settings() {
    const about = await api("about");
    const mode = hasProfile() ? await api("notice_mode") : "full";
    modal(
      "Настройки",
      `<div class="listhead"><strong>Slop Mail</strong><span>Версия ${h(about.version)}</span></div>
      <fieldset><legend>Получение почты</legend><label class="check"><input type="checkbox" id="startup" ${prefs.startup ? "checked" : ""}> При открытии профиля</label>
      <label>Пока приложение открыто<select id="minutes">${options(
        [
          [0, "Только вручную"],
          [1, "Каждую минуту"],
          [5, "Каждые 5 минут"],
          [15, "Каждые 15 минут"],
          [30, "Каждые 30 минут"],
          [60, "Каждый час"],
        ],
        prefs.minutes,
      )}</select></label>
      <label class="check"><input id="instant" type="checkbox" ${prefs.instant ? "checked" : ""} ${prefs.minutes ? "" : "disabled"}> Получать сразу при поддержке IMAP IDLE</label>
      <label class="check"><input id="notify" type="checkbox" ${prefs.notify ? "checked" : ""}> Системные уведомления о новых письмах</label><label>Содержание уведомлений профиля<select id="notice-mode" ${hasProfile() ? "" : "disabled"}>${options(
        [
          ["full", "Ящик, количество и тема"],
          ["account", "Ящик и количество"],
          ["count", "Только количество"],
        ],
        mode,
      )}</select></label></fieldset>
      <fieldset><legend>Шаблоны подключения</legend><select id="presetlist" aria-label="Шаблон">${options(
        prefs.presets.map((x) => [x.id, x.name]),
        null,
      )}</select><div class="actions"><button id="newpreset">Создать</button><button id="editpreset" ${prefs.presets.length ? "" : "disabled"}>Изменить</button><button id="delpreset" ${prefs.presets.length ? "" : "disabled"}>Удалить</button></div></fieldset>
      <fieldset><legend>Резервная копия профиля</legend><p>В копию входят письма, вложения и черновики. Пароли, кеш и внешние файлы черновиков сохраняются отдельно. Восстановление создаёт новый профиль и приостанавливает исходящие письма.</p><button id="backup-save" ${hasProfile() ? "" : "disabled"}>Сохранить копию…</button><button id="backup-open">Восстановить…</button><span id="backup-result" role="status"></span></fieldset>
      <button id="disk-usage">Место на диске</button><button id="changes-show">История изменений</button><button id="mail-rules">Правила для входящих</button><fieldset><legend>Обновления Slop Mail</legend>
      <label>Проверять новые версии<select id="update-hours">${options(
        [
          [0, "Только вручную"],
          [1, "Каждый час"],
          [6, "Каждые 6 часов"],
          [24, "Раз в сутки"],
          [168, "Раз в неделю"],
        ],
        prefs.updateHours,
      )}</select></label>
      <button id="updates">Проверить сейчас</button><div id="updateinfo" role="status"></div></fieldset>`,
    );
    get("startup").onchange = () => {
      prefs.startup = get("startup").checked;
      persist();
    };
    get("minutes").onchange = () => {
      prefs.minutes = Number(get("minutes").value);
      get("instant").disabled = !prefs.minutes;
      persist();
      schedule();
    };
    get("instant").onchange = () => {
      prefs.instant = get("instant").checked;
      persist();
      schedule();
    };
    get("notify").onchange = () => {
      prefs.notify = get("notify").checked;
      persist();
      schedule();
    };
    click("disk-usage", () => showDiskUsage(settings));
    get("notice-mode").onchange = () =>
      api("set_notice_mode", { mode: get("notice-mode").value });
    click("backup-open", restoreBackup);
    click("backup-save", async () => {
      await flush();
      const label = get("backup-result");
      if (await api("save_backup"))
        label.textContent = " Копия проверена и сохранена";
    });
    click("changes-show", () => showChanges(about));
    click("mail-rules", rules);
    click("newpreset", () => edit(null));
    click("editpreset", () =>
      edit(prefs.presets.find((x) => x.id === get("presetlist").value)),
    );
    click("delpreset", async () => {
      const id = get("presetlist").value;
      if (await api("confirm", { text: "Удалить выбранный шаблон?" })) {
        prefs.presets = prefs.presets.filter((x) => x.id !== id);
        persist();
        await settings();
      }
    });
    get("update-hours").onchange = () => {
      prefs.updateHours = Number(get("update-hours").value);
      persist();
      updates.schedule();
    };
    click("updates", async () => {
      const label = get("updateinfo"),
        button = get("updates");
      button.disabled = true;
      label.textContent = "Проверка GitHub Releases…";
      try {
        const version = await updates.check();
        updates.schedule();
        if (!label.isConnected) return;
        if (!version) {
          label.textContent = "Установлена актуальная версия";
          return;
        }
        label.textContent = "Доступна версия " + version;
        if (
          await api("confirm", {
            text:
              "Скачать и установить Slop Mail " +
              version +
              "? Приложение перезапустится.",
          })
        ) {
          await flush();
          label.textContent = "Загрузка и проверка подписи…";
          await window.__TAURI__.core.invoke("update", {
            installVersion: version,
          });
        }
      } finally {
        button.disabled = false;
      }
    });
  }
  function edit(preset) {
    const p = preset || {
      name: "",
      incoming: { proto: "imap", host: "", port: 993, tls: "tls" },
      outgoing: { proto: "smtp", host: "", port: 465, tls: "tls" },
      login: "%EMAILADDRESS%",
      smtp_login: "%EMAILADDRESS%",
    };
    const login = (k, value) =>
      `<label>Логин<select id="${k}login">${options(
        [
          ["%EMAILADDRESS%", "Полный email"],
          ["%EMAILLOCALPART%", "Часть до @"],
        ],
        value,
      )}</select></label>`;
    modal(
      "Шаблон подключения",
      `<label>Название<input id="presetname" value="${h(p.name)}"></label><fieldset><legend>Входящий сервер</legend>${serverFields("in", p.incoming)}${login("in", p.login)}</fieldset><fieldset><legend>Исходящий сервер</legend>${serverFields("out", p.outgoing)}${login("out", p.smtp_login)}</fieldset><button id="savepreset">Сохранить шаблон</button>`,
    );
    bindPorts();
    click("savepreset", async () => {
      const name = get("presetname").value.trim(),
        incoming = endpoint("in"),
        outgoing = endpoint("out");
      if (
        !name ||
        [incoming, outgoing].some(
          (s) =>
            !s.host ||
            !Number.isInteger(s.port) ||
            s.port < 1 ||
            s.port > 65535,
        )
      )
        throw Error("Введите название, серверы и порты от 1 до 65535");
      const item = {
        id: preset?.id || crypto.randomUUID(),
        name,
        incoming,
        outgoing,
        login: get("inlogin").value,
        smtp_login: get("outlogin").value,
      };
      prefs.presets = [...prefs.presets.filter((x) => x.id !== item.id), item];
      persist();
      await settings();
    });
  }
  return settings;
}
