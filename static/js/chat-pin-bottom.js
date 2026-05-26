// Pin the page to its bottom after load so the latest chat turn sits
// just above the fixed composer. Two extra ticks cover Lucide-icon and
// font-load reflows that change layout shortly after DOMContentLoaded.
//
// No-op when neither chat thread is on the page. Loaded with `defer` from
// base.html on every page; the early return keeps non-chat pages free of
// any side effect.
(function () {
  function isChatPage() {
    return !!document.getElementById('story-thread')
        || !!document.getElementById('pitch-thread');
  }
  function pinBottom() {
    window.scrollTo({ top: document.body.scrollHeight, behavior: 'instant' });
  }
  if (!isChatPage()) return;
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', pinBottom);
  } else {
    pinBottom();
  }
  window.setTimeout(pinBottom, 60);
  window.setTimeout(pinBottom, 240);
})();
