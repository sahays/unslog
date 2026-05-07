// recorder.js — small mic-capture island for the active session page.
//
// Markup contract:
//   data-record-button   — button that toggles recording
//   data-record-status   — element whose textContent shows state
//   data-record-target   — textarea filled with the transcript
//   data-record-audio    — hidden input filled with the audio_path
//   data-record-endpoint — POST URL for the multipart audio
//
// Keyboard shortcuts (active session page):
//   r          — toggle record/stop, when focus is not in a form field
//   escape     — stop recording (any focus)
//   cmd/ctrl-enter — submit the answer form, from anywhere on the page

(function () {
  const root = document.querySelector("[data-recorder-root]");
  if (!root) return;

  const btn = root.querySelector("[data-record-button]");
  const status = root.querySelector("[data-record-status]");
  const transcript = document.querySelector("[data-record-target]");
  const audioInput = document.querySelector("[data-record-audio]");
  const endpoint = root.dataset.recordEndpoint;
  if (!btn || !status || !transcript || !audioInput || !endpoint) return;

  let media = null;
  let recorder = null;
  let chunks = [];
  let recording = false;
  let timerHandle = null;
  let recordStartedAt = 0;

  function setState(s) {
    status.textContent = s;
    status.dataset.state = s;
  }

  function startTimer() {
    recordStartedAt = Date.now();
    if (timerHandle) clearInterval(timerHandle);
    timerHandle = setInterval(() => {
      if (!recording) return;
      const sec = Math.floor((Date.now() - recordStartedAt) / 1000);
      const m = Math.floor(sec / 60);
      const s = String(sec % 60).padStart(2, "0");
      setState(`recording… ${m}:${s} (press r or esc to stop)`);
    }, 250);
  }

  function stopTimer() {
    if (timerHandle) {
      clearInterval(timerHandle);
      timerHandle = null;
    }
  }

  async function start() {
    chunks = [];
    try {
      media = await navigator.mediaDevices.getUserMedia({ audio: true });
    } catch (e) {
      setState("mic permission denied");
      return;
    }
    const mime = MediaRecorder.isTypeSupported("audio/webm;codecs=opus")
      ? "audio/webm;codecs=opus"
      : "audio/webm";
    recorder = new MediaRecorder(media, { mimeType: mime });
    recorder.ondataavailable = (ev) => {
      if (ev.data && ev.data.size > 0) chunks.push(ev.data);
    };
    recorder.onstop = onStop;
    recorder.start();
    recording = true;
    btn.textContent = "Stop";
    startTimer();
    setState("recording… 0:00 (press r or esc to stop)");
  }

  async function stop() {
    if (!recorder) return;
    recorder.stop();
    media.getTracks().forEach((t) => t.stop());
    recording = false;
    stopTimer();
    btn.textContent = "Record";
    setState("uploading & transcribing…");
  }

  async function onStop() {
    const blob = new Blob(chunks, { type: chunks[0]?.type || "audio/webm" });
    const fd = new FormData();
    fd.append("file", blob, "answer.webm");

    let resp;
    try {
      resp = await fetch(endpoint, { method: "POST", body: fd });
    } catch (e) {
      setState("network error — try again");
      return;
    }

    if (!resp.ok) {
      const t = await resp.text();
      setState("error: " + (t.slice(0, 80) || resp.status));
      return;
    }

    let data;
    try {
      data = await resp.json();
    } catch (e) {
      setState("bad response from server");
      return;
    }

    transcript.value = data.transcript || "";
    audioInput.value = data.audio_path || "";
    transcript.focus();
    setState("transcribed — review and submit (cmd+enter)");
  }

  btn.addEventListener("click", (ev) => {
    ev.preventDefault();
    if (recording) stop();
    else start();
  });

  // Keyboard shortcuts.
  function isTypingTarget(el) {
    if (!el) return false;
    const tag = el.tagName;
    return tag === "INPUT" || tag === "TEXTAREA" || el.isContentEditable;
  }

  document.addEventListener("keydown", (ev) => {
    // Cmd/Ctrl + Enter — submit the nearest form to the transcript textarea.
    if ((ev.metaKey || ev.ctrlKey) && ev.key === "Enter") {
      const form = transcript.closest("form");
      if (form) {
        ev.preventDefault();
        if (typeof form.requestSubmit === "function") form.requestSubmit();
        else form.submit();
      }
      return;
    }
    // Escape — stop recording, regardless of focus.
    if (ev.key === "Escape" && recording) {
      ev.preventDefault();
      stop();
      return;
    }
    // r — toggle record, only when not typing.
    if (ev.key === "r" && !isTypingTarget(document.activeElement) && !ev.metaKey && !ev.ctrlKey && !ev.altKey) {
      ev.preventDefault();
      if (recording) stop();
      else start();
    }
  });
})();
