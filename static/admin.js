(function () {
  var app = document.getElementById("app");
  var menu = document.querySelector("[data-menu]");
  var scrim = document.querySelector("[data-scrim]");
  function close() { if (!app) return; app.removeAttribute("data-drawer"); if (scrim) scrim.hidden = true; }
  if (menu && app) menu.addEventListener("click", function () {
    var open = app.getAttribute("data-drawer") === "aperto";
    if (open) close(); else { app.setAttribute("data-drawer", "aperto"); if (scrim) scrim.hidden = false; }
  });
  if (scrim) scrim.addEventListener("click", close);
  document.querySelectorAll("[data-copy]").forEach(function (btn) {
    btn.addEventListener("click", function () {
      var area = btn.parentElement && btn.parentElement.querySelector("[data-copy-src]");
      var text = area ? area.value : "";
      if (navigator.clipboard) navigator.clipboard.writeText(text);
    });
  });
  var html = document.querySelector("[data-html]");
  var frame = document.querySelector("[data-preview]");
  function paint(width) { if (!html || !frame) return; frame.srcdoc = html.value; if (width) frame.style.width = width + "px"; }
  if (html && frame) {
    html.addEventListener("input", function () { paint(); });
    document.querySelectorAll("[data-width]").forEach(function (b) { b.addEventListener("click", function () { paint(b.getAttribute("data-width")); }); });
  }
  document.querySelectorAll("[data-confirm]").forEach(function (form) {
    form.addEventListener("submit", function (e) { if (!window.confirm(form.getAttribute("data-confirm"))) e.preventDefault(); });
  });
})();
