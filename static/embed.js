(function () {
  var s = document.currentScript;
  if (!s) return;
  var origin = new URL(s.src, window.location.href).origin;
  var frame = document.createElement("iframe");
  frame.src = origin + "/modulo";
  frame.title = "Iscrizione";
  frame.style.width = "100%";
  frame.style.border = "0";
  frame.style.minHeight = "480px";
  s.parentNode.insertBefore(frame, s.nextSibling);
  window.addEventListener("message", function (ev) {
    if (ev.origin !== origin) return;
    if (!ev.data || ev.data.type !== "iscrizioni-height") return;
    var h = Number(ev.data.height);
    if (h > 200 && h < 8000) frame.style.height = (h + 8) + "px";
  });
})();
