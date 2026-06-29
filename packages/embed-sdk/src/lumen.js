/**
 * Lumen embed SDK (MVP stub).
 *
 *   <div id="report"></div>
 *   <script src="https://cdn.your-host/lumen.js"></script>
 *   <script>
 *     Lumen.render("#report", {
 *       baseUrl: "https://embed.your-host",
 *       dashboard: "sales",
 *       token: "<jwt minted by your backend>",
 *     });
 *   </script>
 *
 * Today this wraps the iframe embedding path. The roadmap (see docs/DESIGN.md)
 * replaces it with a <analytics-dashboard> Web Component and a postMessage
 * resize/event channel — without changing this call signature.
 */
(function (global) {
  "use strict";

  function render(target, opts) {
    var el = typeof target === "string" ? document.querySelector(target) : target;
    if (!el) throw new Error("Lumen.render: target not found: " + target);
    var base = (opts.baseUrl || "").replace(/\/$/, "");
    var src =
      base +
      "/embed/dashboard/" +
      encodeURIComponent(opts.dashboard) +
      "?token=" +
      encodeURIComponent(opts.token);

    var iframe = document.createElement("iframe");
    iframe.src = src;
    iframe.title = opts.title || "Lumen dashboard";
    iframe.style.width = opts.width || "100%";
    iframe.style.height = opts.height || "100%";
    iframe.style.border = "0";
    iframe.loading = "lazy";
    el.appendChild(iframe);
    return iframe;
  }

  global.Lumen = { render: render, version: "0.1.0" };
})(typeof window !== "undefined" ? window : this);
