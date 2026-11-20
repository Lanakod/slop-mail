import { escapeHtml as h } from "./helpers.js";
export const roles = { to: "Кому", cc: "Копия", bcc: "Скрытая копия" };
export function recipientFields() {
  return Object.entries(roles)
    .map(
      ([role, label]) =>
        `<div class="recipient-row"><label>${label}<input data-field="${role}" placeholder="Имя или email" autocomplete="off"></label><div data-chips="${role}" class="chips"></div></div>`,
    )
    .join("");
}
export function recipientEditor(host, initial, people, api, save, action) {
  const q = (r) => host.querySelector('[data-field="' + r + '"]');
  const chips = { to: [], cc: [], bcc: [] };
  function draw() {
    for (const role of Object.keys(roles)) {
      const area = host.querySelector('[data-chips="' + role + '"]');
      area.replaceChildren();
      chips[role].forEach((address, i) => {
        const item = document.createElement("span");
        item.className = "chip";
        item.innerHTML = `<span>${h(address)}</span><select aria-label="Перенести адрес">${Object.entries(
          roles,
        )
          .map(
            ([k, label]) =>
              `<option value="${k}" ${k === role ? "selected" : ""}>${label}</option>`,
          )
          .join(
            "",
          )}</select><button type="button" aria-label="Убрать адрес">×</button>`;
        item.querySelector("select").onchange = (e) => {
          const next = item.querySelector("select").value;
          chips[role].splice(i, 1);
          chips[next].push(address);
          draw();
          save();
        };
        item.querySelector("button").onclick = () => {
          chips[role].splice(i, 1);
          draw();
          save();
        };
        area.append(item);
      });
    }
  }
  function values() {
    return Object.fromEntries(
      Object.keys(roles).map((k) => [
        k,
        [...chips[k], q(k).value.trim()].filter(Boolean).join(", "),
      ]),
    );
  }
  let revision = 0;
  async function commit(role) {
    const input = q(role),
      text = input.value.trim().replace(/[,;]+$/, "");
    if (!text) return;
    const version = ++revision;
    const parsed = await api("preview", { to: text });
    if (
      !host.isConnected ||
      version !== revision ||
      input.value.trim().replace(/[,;]+$/, "") !== text
    )
      return;
    for (const email of parsed) {
      if (
        !Object.values(chips)
          .flat()
          .some((v) => v.toLowerCase() === email.toLowerCase())
      )
        chips[role].push(email);
    }
    input.value = "";
    draw();
    q("suggest").replaceChildren();
    save();
  }
  for (const k of Object.keys(roles)) {
    q(k).value = initial[k] || "";
    q(k).onkeydown = action(async (e) => {
      if (e.key === "Enter" && !e.ctrlKey && !e.metaKey) {
        e.preventDefault();
        await commit(k);
      }
    });
    q(k).onblur = () => {
      if (q(k).value.trim()) commit(k).catch(() => {});
    };
    q(k).oninput = () => {
      revision++;
      let term = q(k).value.split(",").at(-1).trim().toLowerCase();
      q("suggest").replaceChildren();
      if (!term) return;
      for (const person of people) {
        for (const email of person.emails.split(", ")) {
          if (
            q("suggest").children.length >= 6 ||
            !(person.name + " " + email).toLowerCase().includes(term)
          )
            continue;
          const b = document.createElement("button");
          b.textContent = person.name + " · " + email;
          b.onmousedown = (e) => e.preventDefault();
          b.onclick = action(async () => {
            const seg = q(k).value.split(",");
            seg.pop();
            q(k).value = [...seg, email].join(",");
            await commit(k);
            q(k).focus();
          });
          q("suggest").append(b);
        }
      }
    };
  }
  // Stored valid address lists open as chips. Keep unfinished text editable.
  async function restore() {
    for (const k of Object.keys(roles)) {
      const raw = q(k).value;
      if (!raw) continue;
      try {
        const parsed = await api("preview", { to: raw });
        if (host.isConnected && q(k).value === raw) {
          chips[k] = parsed;
          q(k).value = "";
          draw();
        }
      } catch {
        /* incomplete draft */
      }
    }
  }
  draw();
  restore();
  return values;
}
