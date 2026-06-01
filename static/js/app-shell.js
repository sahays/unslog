// app-shell.js — global page-shell behaviors used on every route.
//
// Loads after htmx + alpine + lucide. All four behaviors are independent:
//   • Lucide icon initialization (on load + after every htmx swap)
//   • Sidebar active-route highlight (URL-based, longest-prefix match)
//   • YouTube-style top progress bar for navigations + htmx requests
//   • Modal overlay for forms that opt in via data-loading-{title,detail}
//
// Was previously inlined in base.html; pulled out so it's lintable and
// debuggable as JS. Behavior is unchanged.

(function () {
  /* ── Lucide ──────────────────────────────────────────── */
  function renderIcons() {
    if (window.lucide && typeof window.lucide.createIcons === 'function') {
      window.lucide.createIcons();
    }
  }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', renderIcons);
  } else {
    renderIcons();
  }
  document.body.addEventListener('htmx:afterSwap', renderIcons);
  document.body.addEventListener('htmx:load', renderIcons);

  /* ── Sidebar active-route highlight (URL-based fallback until
       SSR nav_active is wired in Phase 3). Matches the longest
       data-nav-key prefix against the current pathname. */
  function markActiveNav() {
    var path = window.location.pathname;
    var links = document.querySelectorAll('.nav-link-side[data-nav-key]');
    var best = null;
    var bestLen = -1;
    links.forEach(function (a) {
      try {
        var href = new URL(a.getAttribute('href'), window.location.origin).pathname;
        if (path === href || path.startsWith(href + '/')) {
          if (href.length > bestLen) { best = a; bestLen = href.length; }
        }
      } catch (_) {}
      a.classList.remove('active');
    });
    if (best) best.classList.add('active');
  }
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', markActiveNav);
  } else {
    markActiveNav();
  }

  /* ── YouTube-style progress bar ──────────────────────── */
  var bar = document.getElementById('progress-bar');
  var inFlight = false;
  function startProgress() {
    if (!bar || inFlight) return;
    inFlight = true;
    bar.style.transition = 'none';
    bar.style.width = '0%';
    bar.style.opacity = '1';
    /* force reflow so the next transition takes effect */
    void bar.offsetWidth;
    bar.style.transition = 'width 8s cubic-bezier(0.05, 0.55, 0.4, 0.95)';
    bar.style.width = '85%';
  }
  function finishProgress() {
    if (!bar) return;
    inFlight = false;
    bar.style.transition = 'width 220ms ease-out, opacity 360ms ease-out 160ms';
    bar.style.width = '100%';
    window.setTimeout(function () { bar.style.opacity = '0'; }, 240);
    window.setTimeout(function () {
      bar.style.transition = 'none';
      bar.style.width = '0%';
    }, 740);
  }

  /* Same-tab link click → start */
  document.addEventListener('click', function (e) {
    var a = e.target.closest('a[href]');
    if (!a) return;
    if (a.target && a.target !== '_self') return;
    if (a.hasAttribute('download')) return;
    if (e.metaKey || e.ctrlKey || e.shiftKey || e.altKey || e.button !== 0) return;
    var url;
    try { url = new URL(a.href, window.location.href); } catch (_) { return; }
    if (url.origin !== window.location.origin) return;
    /* same-page anchor jump → no nav, skip */
    if (url.pathname === window.location.pathname && url.search === window.location.search && url.hash) return;
    startProgress();
  }, true);

  /* ── Modal overlay (long form submits) ──────────────── */
  var overlay = document.getElementById('app-overlay');
  var overlayTitle = document.getElementById('app-overlay-title');
  var overlayDetail = document.getElementById('app-overlay-detail');
  function showOverlay(title, detailHTML) {
    if (!overlay) return;
    if (overlayTitle) overlayTitle.textContent = title || 'Working…';
    if (overlayDetail) overlayDetail.innerHTML = detailHTML || '';
    overlay.classList.add('visible');
    overlay.setAttribute('aria-hidden', 'false');
  }
  function hideOverlay() {
    if (!overlay) return;
    overlay.classList.remove('visible');
    overlay.setAttribute('aria-hidden', 'true');
  }

  /* Plain form submit → start */
  document.addEventListener('submit', function (e) {
    var form = e.target;
    if (!(form instanceof HTMLFormElement)) return;
    if (form.target && form.target !== '_self') return;
    startProgress();
    /* Submit-in-flight button feedback: spin the icon, dim the button
       while the next page loads. FormData is already collected at this
       point, so flipping disabled is safe. */
    if (form.dataset.noLoading !== 'true') {
      var btn = form.querySelector('button[type="submit"], input[type="submit"]');
      if (btn) {
        btn.classList.add('btn-loading');
        btn.setAttribute('aria-busy', 'true');
      }
    }
    /* Long-running form? Show modal overlay with operation-specific copy. */
    if (form.dataset.loadingTitle || form.dataset.loadingDetail) {
      showOverlay(
        form.dataset.loadingTitle || 'Working…',
        form.dataset.loadingDetail || ''
      );
    }
  }, true);

  /* htmx requests */
  document.body.addEventListener('htmx:beforeRequest', startProgress);
  document.body.addEventListener('htmx:afterRequest', finishProgress);

  /* ── CSRF header for multipart form submits ──────────────────────────
       The csrf_verify middleware accepts the token from either the
       form-encoded body or the X-CSRF-Token header. Multipart bodies
       can't be cheaply scanned server-side, so we intercept any form
       tagged data-csrf-multipart, attach the header by hand, and POST
       via fetch. The response is rendered into the document (matching
       a normal browser submit) and Location-redirects are followed. */
  function readCsrfCookie() {
    var raw = document.cookie.split('; ').find(function (r) { return r.startsWith('__Host-csrf='); });
    return raw ? raw.split('=').slice(1).join('=') : '';
  }
  document.addEventListener('submit', function (e) {
    var form = e.target;
    if (!(form instanceof HTMLFormElement)) return;
    if (form.dataset.csrfMultipart === undefined) return;
    e.preventDefault();
    var fd = new FormData(form);
    fetch(form.action, {
      method: form.method || 'POST',
      body: fd,
      headers: { 'X-CSRF-Token': readCsrfCookie() },
      credentials: 'same-origin',
      redirect: 'follow',
    }).then(function (resp) {
      /* Server typically 303s to /assets; fetch follows it, and the body
         is the new page HTML. Replace the document content to mirror a
         classic browser submit. */
      if (resp.redirected) { window.location.href = resp.url; return; }
      return resp.text().then(function (html) {
        document.open();
        document.write(html);
        document.close();
      });
    }).catch(function () {
      window.location.reload();
    });
  }, true);

  /* Page loaded (initial nav or bfcache restore) → finish + clear overlay */
  window.addEventListener('pageshow', function () {
    finishProgress();
    hideOverlay();
  });
})();
