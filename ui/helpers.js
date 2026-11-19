export function escapeHtml(value) {
  return String(value ?? "").replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ],
  );
}
export function splitQuote(body) {
  const offset = body.search(
    /^(?:>|On .+ wrote:|.*написал(?:а)?[:(]|-{3,}\s*Original Message)/im,
  );
  return offset === -1
    ? [body, ""]
    : [body.slice(0, offset), body.slice(offset)];
}
export function options(items, value) {
  return items
    .map(
      ([id, name]) =>
        `<option value="${escapeHtml(id)}" ${id === value ? "selected" : ""}>${escapeHtml(name)}</option>`,
    )
    .join("");
}
export function basename(path) {
  return path.split(/[\\/]/).pop();
}
export function draftBody(draft) {
  return draft.body + (draft.signature ? "\n\n-- \n" + draft.signature : "");
}
