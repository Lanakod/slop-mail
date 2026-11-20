import { escapeHtml as h } from "./helpers.js";

export function threadList({ api, action, date, open, refresh, bulk }) {
  const expanded = new Set();
  let menu = null;
  function closeMenu() {
    menu?.remove();
    menu = null;
  }
  document.addEventListener("pointerdown", (e) => {
    if (!menu?.contains(e.target)) closeMenu();
  });
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && menu) {
      e.preventDefault();
      closeMenu();
    }
  });
  window.addEventListener("blur", closeMenu);
  function render(host, rows, filters, activeId, profile) {
    closeMenu();
    host.replaceChildren();
    const selected = new Set();
    let lastSelected = null;
    const targets = new WeakMap();
    const visible = () => Array.from(host.querySelectorAll(".mail-row"));
    function context(event, node, m, group) {
      event.preventDefault();
      closeMenu();
      menu = document.createElement("div");
      menu.className = "mail-menu";
      menu.setAttribute("role", "menu");
      const title = document.createElement("small");
      const chosen = selected.has(node)
        ? visible().filter((b) => selected.has(b))
        : [node];
      title.textContent =
        chosen.length > 1
          ? "Выбрано строк: " + chosen.length
          : group
            ? "Письма ветки в этом списке"
            : "Письмо";
      menu.append(title);
      for (const [key, name] of [
        ["read", "Отметить прочитанным"],
        ["unread", "Отметить непрочитанным"],
        ["star", "Поставить звезду"],
        ["archive", "В архив"],
        ["trash", "В корзину"],
        ["move", "Переместить в папку…"],
        ["snooze", "Отложить до…"],
        ["wake", "Вернуть сейчас"],
      ]) {
        if (key === "wake" && filters.view !== "snoozed") continue;
        const b = document.createElement("button");
        b.type = "button";
        b.setAttribute("role", "menuitem");
        b.textContent = name;
        b.onclick = action(async () => {
          closeMenu();
          const batches = await Promise.all(
            chosen.map((b) => {
              const { m, group } = targets.get(b);
              return group
                ? api("list", { ...filters, branch: m.thread }, profile)
                : [m];
            }),
          );
          const ids = [...new Set(batches.flat().map((row) => row.id))];
          if (chosen.length === 1 && ["read", "unread"].includes(key)) {
            await api(
              "mark_seen",
              { thread: m.thread, ids, seen: key === "read" },
              profile,
            );
            await refresh();
          } else await bulk(key, ids, profile);
        });
        menu.append(b);
      }
      document.body.append(menu);
      const rect = node.getBoundingClientRect();
      const x = event.type === "keydown" ? rect.left : event.clientX,
        y = event.type === "keydown" ? rect.bottom : event.clientY;
      menu.style.left =
        Math.max(0, Math.min(x, window.innerWidth - menu.offsetWidth)) + "px";
      menu.style.top =
        Math.max(0, Math.min(y, window.innerHeight - menu.offsetHeight)) + "px";
      menu.querySelector("button").focus();
    }
    function message(m, group = false) {
      const b = document.createElement("button");
      b.type = "button";
      b.className =
        "thread mail-row" +
        ((group ? m.unread : !m.seen) ? " unread" : "") +
        (m.id === activeId ? " active" : "");
      b.innerHTML = `<time>${h(date(m.stamp))}</time><strong>${h((group ? m.topic : m.subject) || "Без темы")}</strong><span class="meta">${h(m.sender)} → ${h(m.recipients)}</span><small>${h(m.preview)}</small>`;
      if (m.labels) {
        const tag = document.createElement("small");
        tag.textContent = m.labels;
        b.append(tag);
      }
      targets.set(b, { m, group });
      if (m.id === activeId) lastSelected = b;
      b.onclick = action(async (e) => {
        if (e?.ctrlKey || e?.metaKey || e?.shiftKey) {
          const rows = visible();
          if (e.shiftKey && rows.includes(lastSelected)) {
            const range = [rows.indexOf(lastSelected), rows.indexOf(b)].sort(
              (a, b) => a - b,
            );
            for (const row of rows.slice(range[0], range[1] + 1))
              selected.add(row);
          } else if (selected.has(b)) selected.delete(b);
          else selected.add(b);
          lastSelected = b;
          for (const row of rows)
            row.classList.toggle("selected", selected.has(row));
        } else await open(m.thread, m.id);
      });
      b.oncontextmenu = (e) => context(e, b, m, group);
      b.onkeydown = (e) => {
        if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10"))
          context(e, b, m, group);
      };
      return b;
    }
    for (const m of rows) {
      if (m.total <= 1) {
        host.append(message(m));
        continue;
      }
      const section = document.createElement("section"),
        head = document.createElement("div"),
        toggle = document.createElement("button"),
        children = document.createElement("div");
      head.className = "branch-head";
      toggle.type = "button";
      toggle.className = "branch-toggle";
      children.className = "branch-children";
      head.append(toggle, message(m, true));
      section.append(head, children);
      host.append(section);
      let version = 0;
      async function show() {
        const request = ++version,
          on = expanded.has(m.thread);
        toggle.textContent = (on ? "▾ " : "▸ ") + m.total;
        toggle.setAttribute("aria-expanded", String(on));
        toggle.setAttribute(
          "aria-label",
          (on ? "Свернуть" : "Раскрыть") +
            " переписку: " +
            (m.topic || "Без темы") +
            ", писем: " +
            m.total,
        );
        children.hidden = !on;
        children.replaceChildren();
        if (on) {
          const data = await api(
            "list",
            { ...filters, branch: m.thread },
            profile,
          );
          if (children.isConnected && request === version)
            children.replaceChildren(...data.map((row) => message(row)));
        }
      }
      toggle.onclick = action(async () => {
        if (expanded.has(m.thread)) expanded.delete(m.thread);
        else expanded.add(m.thread);
        await show();
      });
      action(show)();
    }
  }
  return {
    render,
    reset() {
      expanded.clear();
      closeMenu();
    },
    closeMenu,
  };
}
