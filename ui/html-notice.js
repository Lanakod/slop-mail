// Rendering the local HTML and fetching an image from the sender are separate choices.
export function htmlNotice(host, render, action) {
  const notice = document.createElement("div");
  notice.className = "html-notice";
  const message = document.createElement("span");
  message.textContent =
    "В целях безопасности загрузка содержимого из Интернета заблокирована.";
  const button = document.createElement("button");
  button.textContent = "Отобразить содержимое";
  let shown = false;
  button.onclick = action(async () => {
    if (button.disabled) return;
    button.disabled = true;
    try {
      const hasRemoteImages = await render(shown);
      if (shown) {
        message.textContent =
          "Для этого письма разрешена загрузка внешних изображений.";
        button.hidden = true;
      } else {
        shown = true;
        button.textContent = "Загрузить внешние изображения";
        button.hidden = !hasRemoteImages;
        if (!hasRemoteImages)
          message.textContent =
            "HTML письма показан. Загрузка из Интернета заблокирована.";
      }
    } finally {
      button.disabled = false;
    }
  });
  notice.append(message, button);
  host.before(notice);
}
