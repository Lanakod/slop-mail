import { escapeHtml as h } from "./helpers.js";

function size(bytes) {
  const units = ["Б", "КиБ", "МиБ", "ГиБ", "ТиБ"];
  let value = bytes,
    unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return (
    value.toLocaleString("ru", { maximumFractionDigits: 1 }) + " " + units[unit]
  );
}

export function diskReport(report) {
  const accounts = report.accounts
    .map(
      (
        a,
      ) => `<tr><th scope="row">${h(a.name || a.address)}${a.name ? "<small>" + h(a.address) + "</small>" : ""}</th>
      <td>${size(a.text_bytes)}</td><td>${size(a.file_bytes)}</td><td>${size(a.queue_bytes)}</td>
      <td><strong>${size(a.text_bytes + a.file_bytes + a.queue_bytes)}</strong></td></tr>`,
    )
    .join("");
  const messages = report.largest
    .map(
      (
        m,
        index,
      ) => `<li><button data-message="${index}"><span><strong>${h(m.subject || "Без темы")}</strong>
      <small>${h(m.address)} · ${h(m.folder)}</small></span><b>${size(m.bytes)}</b></button></li>`,
    )
    .join("");
  return `<div class="disk-totals"><div>База профиля<strong>${size(report.database_bytes)}</strong></div>
      <div>Повторные копии вложений<strong>${size(report.duplicates)}</strong></div></div>
      <p class="hint">Размер базы включает индекс поиска и служебные файлы SQLite. Для одинаковых вложений во всех ящиках профиля первая копия исключена из объёма повторов.</p>
      <h3>Открытые вложения · ${size(report.opened.bytes)}</h3><p>Самой старой копии ${report.opened.oldest} дн.</p><button id="clean-opened">Очистить неизменённые копии старше 7 дней</button><h3>Почта по ящикам</h3>${accounts ? '<div class="disk-table"><table><thead><tr><th>Ящик</th><th>Текст и HTML</th><th>Вложения</th><th>Очередь SMTP</th><th>Итого</th></tr></thead><tbody>' + accounts + "</tbody></table></div>" : "<p>Почтовых ящиков пока нет.</p>"}
      <h3>Самые тяжёлые письма</h3>${messages ? '<ol class="disk-messages">' + messages + "</ol>" : "<p>В профиле пока нет писем.</p>"}`;
}
