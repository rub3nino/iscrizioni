(function () {
  function send() {
    var h = document.documentElement.scrollHeight;
    if (window.parent && window.parent !== window) parent.postMessage({ type: "iscrizioni-height", height: h }, "*");
  }
  send();
  window.addEventListener("load", send);
  if (window.ResizeObserver) new ResizeObserver(send).observe(document.body);
  var node = document.querySelector("[data-countdown]");
  if (!node) return;
  var target = Date.parse(node.getAttribute("datetime") || "");
  if (!target) return;
  function tick() {
    var left = target - Date.now();
    if (left <= 0) return;
    var s = Math.floor(left / 1000);
    var d = Math.floor(s / 86400); s -= d * 86400;
    var h = Math.floor(s / 3600); s -= h * 3600;
    var m = Math.floor(s / 60); s -= m * 60;
    var pad = function (n) { return (n < 10 ? "0" : "") + n; };
    node.textContent = (d ? d + " g " : "") + pad(h) + ":" + pad(m) + ":" + pad(s);
  }
  if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches) setInterval(tick, 1000);
  tick();
})();
