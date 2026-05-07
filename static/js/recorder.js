// recorder.js — small mic-capture island for the active session page.
//
// Markup contract:
//   data-record-button   — button that toggles recording
//   data-record-status   — element whose textContent shows state
//   data-record-target   — textarea filled with the transcript
//   data-record-audio    — hidden input filled with the audio_path
//   data-record-endpoint — POST URL for the multipart audio
//
// The page should set the data-record-endpoint attribute on the recorder
// container element (a <div> wrapping the controls).

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

  function setState(s) {
    status.textContent = s;
    status.dataset.state = s;
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
    setState("recording…");
  }

  async function stop() {
    if (!recorder) return;
    recorder.stop();
    media.getTracks().forEach((t) => t.stop());
    recording = false;
    btn.textContent = "Record";
    setState("uploading…");
  }

  async function onStop() {
    const blob = new Blob(chunks, { type: chunks[0]?.type || "audio/webm" });
    const fd = new FormData();
    // Always send a .webm extension so the server picks the right STT format.
    fd.append("file", blob, "answer.webm");

    let resp;
    try {
      resp = await fetch(endpoint, { method: "POST", body: fd });
    } catch (e) {
      setState("network error");
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
      setState("bad response");
      return;
    }

    transcript.value = data.transcript || "";
    audioInput.value = data.audio_path || "";
    setState("transcribed — review and submit");
  }

  btn.addEventListener("click", (ev) => {
    ev.preventDefault();
    if (recording) stop();
    else start();
  });
})();
