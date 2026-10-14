import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type CallMeta = {
  id: string;
  started_unix: number;
  duration_secs: number;
  line_count: number;
  preview: string;
};

type Entry = {
  id: string;
  at_unix_ms: number;
  role: "you" | "them" | "coach";
  text: string;
  intent: string;
};

type CallDetail = {
  meta: CallMeta;
  transcript: string;
  entries: Entry[];
  brief: PostCallBrief;
};

type PostCallBrief = {
  about: string;
  key_points: string[];
  next_step: string;
  user_note: string;
  generated: boolean;
};

type View = "sessions" | "history" | "transcript" | "settings";

const status = document.querySelector("#status") as HTMLElement;
const pageTitle = document.querySelector("#page-title") as HTMLElement;
const startCall = document.querySelector("#start-call") as HTMLButtonElement;
const stopCall = document.querySelector("#stop-call") as HTMLButtonElement;
const showOverlayBtn = document.querySelector(
  "#show-overlay",
) as HTMLButtonElement;
const historyRows = document.querySelector("#history-rows") as HTMLElement;
const historyWrap = document.querySelector("#history-wrap") as HTMLElement;
const historyEmpty = document.querySelector("#history-empty") as HTMLElement;
const historySearch = document.querySelector(
  "#history-search",
) as HTMLInputElement;
const dgKey = document.querySelector("#dg-key") as HTMLInputElement;
const openrouterKey = document.querySelector(
  "#openrouter-key",
) as HTMLInputElement;
const openrouterModel = document.querySelector(
  "#openrouter-model",
) as HTMLInputElement;
const autoAnswer = document.querySelector("#auto-answer") as HTMLInputElement;
const ovTheme = document.querySelector("#ov-theme") as HTMLSelectElement;
const ovFont = document.querySelector("#ov-font") as HTMLInputElement;
const ovFontLabel = document.querySelector("#ov-font-label") as HTMLElement;
const transcriptTitle = document.querySelector(
  "#transcript-title",
) as HTMLElement;
const transcriptSub = document.querySelector("#transcript-sub") as HTMLElement;
const transcriptBody = document.querySelector(
  "#transcript-body",
) as HTMLElement;
const aiStatus = document.querySelector("#ai-status") as HTMLElement;
const youMeter = document.querySelector("#you-meter") as HTMLElement;
const themMeter = document.querySelector("#them-meter") as HTMLElement;
const youLevel = document.querySelector("#you-level") as HTMLElement;
const themLevel = document.querySelector("#them-level") as HTMLElement;
const youWave = document.querySelector("#you-wave") as HTMLCanvasElement;
const themWave = document.querySelector("#them-wave") as HTMLCanvasElement;
const audioHint = document.querySelector("#audio-hint") as HTMLElement;
const prospectName = document.querySelector(
  "#prospect-name",
) as HTMLInputElement;
const callObjective = document.querySelector(
  "#call-objective",
) as HTMLSelectElement;
const prospectNotes = document.querySelector(
  "#prospect-notes",
) as HTMLTextAreaElement;
const meetingTopicField = document.querySelector(
  "#meeting-topic-field",
) as HTMLElement;
const meetingTopic = document.querySelector(
  "#meeting-topic",
) as HTMLInputElement;
const briefAbout = document.querySelector("#brief-about") as HTMLTextAreaElement;
const briefPoints = document.querySelector(
  "#brief-points",
) as HTMLTextAreaElement;
const briefNext = document.querySelector("#brief-next") as HTMLInputElement;
const briefNote = document.querySelector("#brief-note") as HTMLTextAreaElement;
const briefStatus = document.querySelector("#brief-status") as HTMLElement;
const briefSaved = document.querySelector("#brief-saved") as HTMLElement;
const briefCard = document.querySelector("#brief-card") as HTMLElement;

const views: Record<View, HTMLElement> = {
  sessions: document.querySelector("#view-sessions") as HTMLElement,
  history: document.querySelector("#view-history") as HTMLElement,
  transcript: document.querySelector("#view-transcript") as HTMLElement,
  settings: document.querySelector("#view-settings") as HTMLElement,
};
const nav = {
  sessions: document.querySelector("#nav-sessions") as HTMLButtonElement,
  history: document.querySelector("#nav-history") as HTMLButtonElement,
  settings: document.querySelector("#nav-settings") as HTMLButtonElement,
};

const OVERLAY_PREFS = "sonic-overlay-prefs";
const AUTO_KEY = "sonic-auto-answer";
const PROSPECT_KEY = "sonic-prospect-brief";
const BINS = 36;
const youHist = new Array(BINS).fill(0.05);
const themHist = new Array(BINS).fill(0.05);
let live = false;
let calls: CallMeta[] = [];
let selectedCall = "";
let backTo: View = "sessions";
let listenTarget: "mic" | "sys" | null = null;
let activeView: View = "sessions";
let micDetected = false;
let systemDetected = false;
let justFinished = false;
let previousConnections = {
  openaiKey: "",
  openaiModel: "",
  googleApiKey: "",
  googleModel: "",
};

function openSetup(section = "connections") {
  showView("settings");
  document.querySelectorAll<HTMLElement>(".setup-panel").forEach((panel) => {
    panel.classList.toggle("hidden", panel.id !== `setup-${section}`);
  });
  document.querySelectorAll<HTMLButtonElement>(".setup-tab").forEach((tab) => {
    const selected = tab.dataset.setup === section;
    tab.classList.toggle("active", selected);
    if (selected) tab.setAttribute("aria-current", "page");
    else tab.removeAttribute("aria-current");
  });
}

function updateReadiness() {
  const k = keys();
  const captions = !!k.deepgramKey;
  const coach = !!k.openrouterKey && !!k.openrouterModel;
  const set = (id: string, ready: boolean) => {
    const el = document.getElementById(id)!;
    el.textContent = ready ? "✓" : "○";
    el.classList.toggle("ready", ready);
  };
  set("captions-dot", captions);
  set("coach-dot", coach);
  set("audio-dot", micDetected && systemDetected);
  document.getElementById("captions-ready")!.textContent = captions
    ? "Key added · checked when you start"
    : "Add a captions key to begin";
  document.getElementById("coach-ready")!.textContent = coach
    ? "Key added · checked when you start"
    : k.openrouterKey
      ? "Enter a model ID in Connections"
      : "Optional · captions work without it";
  document.getElementById("audio-ready")!.textContent =
    micDetected && systemDetected
      ? "Both sources detected"
      : micDetected
        ? "Microphone detected · check call audio"
        : systemDetected
          ? "Call audio detected · check microphone"
          : "Test both sources before your call";
  startCall.textContent = captions
    ? "Start coaching →"
    : "Set up live captions →";
}

function overlayPrefs() {
  return {
    theme: ovTheme.value === "dark" ? "dark" : "light",
    font_scale: Number(ovFont.value) / 100,
  };
}

function syncOverlayLabels() {
  ovFontLabel.textContent = `${ovFont.value}%`;
}

function applySavedOverlay(saved: {
  theme?: string;

  font_scale?: number;
}) {
  ovTheme.value = saved.theme === "dark" ? "dark" : "light";
  ovFont.value = String(Math.round((saved.font_scale ?? 1) * 100));

  syncOverlayLabels();
}

async function pushOverlayPrefs() {
  await invoke("set_overlay_prefs", { prefs: overlayPrefs() });
}

function keys() {
  return {
    ...previousConnections,
    deepgramKey: dgKey.value.trim(),
    openrouterKey: openrouterKey.value.trim(),
    openrouterModel: openrouterModel.value.trim(),
    aiProvider: "openrouter",
    overlay: overlayPrefs(),
  };
}

function updateConnectionStatus() {
  const k = keys();
  aiStatus.textContent = !k.deepgramKey
    ? "Add a Deepgram API key to begin."
    : k.openrouterKey && !k.openrouterModel
      ? "Enter a model ID to enable reply suggestions."
      : k.openrouterKey
        ? "Keys and model added · checked when you start"
        : "Captions ready · OpenRouter is optional";
  aiStatus.className = `badge ${k.deepgramKey ? "live" : "ready"}`;
  updateReadiness();
}

function setStatus(text: string, isLive = false) {
  status.textContent = text;
  status.classList.toggle("live", isLive);
}

function showError(error: unknown) {
  setStatus(error instanceof Error ? error.message : String(error), live);
}

function showView(name: View) {
  activeView = name;
  document.querySelector(".main")!.scrollTop = 0;
  (Object.keys(views) as View[]).forEach((key) => {
    views[key].classList.toggle("hidden", key !== name);
  });
  (Object.keys(nav) as Array<keyof typeof nav>).forEach((key) => {
    const selected = key === name || (name === "transcript" && key === backTo);
    nav[key].classList.toggle("active", selected);
    if (selected) nav[key].setAttribute("aria-current", "page");
    else nav[key].removeAttribute("aria-current");
  });
  pageTitle.textContent =
    name === "settings"
      ? "Setup"
      : name === "history"
        ? "Call library"
        : name === "transcript"
          ? "Call review"
          : live
            ? "Live call"
            : "New call";
}

function formatDate(unix: number) {
  return new Date(unix * 1000).toLocaleString([], {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function formatDuration(secs: number) {
  if (!secs) {
    return "—";
  }
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return m ? `${m}m ${s}s` : `${s}s`;
}

function escapeHtml(value: string) {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function callName(meta: CallMeta) {
  const when = new Date(meta.started_unix * 1000).toLocaleString([], {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
  return meta.preview
    ? `${when} · ${meta.preview.slice(0, 44)}`
    : `Call · ${when}`;
}

function rowHtml(meta: CallMeta) {
  const preview = meta.preview || "No spoken captions";
  return `<tr class="call-row" data-call="${escapeHtml(meta.id)}" tabindex="0" role="button" aria-label="Review call from ${escapeHtml(formatDate(meta.started_unix))}">
    <td><b>Call · ${escapeHtml(formatDate(meta.started_unix))}</b><span class="call-preview">${escapeHtml(preview)}</span></td>
    <td>${escapeHtml(formatDate(meta.started_unix))}</td>
    <td>${formatDuration(meta.duration_secs)}</td>
    <td><button class="btn btn-sm" data-call="${escapeHtml(meta.id)}" type="button" tabindex="-1">Review →</button></td>
  </tr>`;
}

function setLiveUi() {
  document.getElementById("prepare-panel")!.classList.toggle("hidden", live);
  document.getElementById("live-panel")!.classList.toggle("hidden", !live);
  document
    .getElementById("saved-banner")!
    .classList.toggle("hidden", !justFinished || live || !calls.length);
  document.getElementById("session-heading")!.textContent = live
    ? "Be present. Your coach is listening."
    : "Go into your call prepared.";
  document.getElementById("session-description")!.textContent = live
    ? "Read the next line here, or open the floating coach above your calling app."
    : "A little context is all your coach needs. You lead the conversation.";
  document.getElementById("session-eyebrow")!.textContent = live
    ? "LIVE CALL"
    : "YOUR NEXT CONVERSATION";
  nav.sessions.innerHTML = `<span aria-hidden="true">${live ? "◉" : "＋"}</span> ${live ? "Live call" : "New call"}`;
  if (activeView === "sessions")
    pageTitle.textContent = live ? "Live call" : "New call";
  document.getElementById("live-prospect")!.textContent =
    prospectName.value || "Current conversation";
  [prospectName, callObjective, prospectNotes, meetingTopic].forEach((field) => {
    field.disabled = live;
  });
  updateReadiness();
}

function render() {
  const query = historySearch.value.trim().toLowerCase();
  const filtered = calls.filter(
    (meta) =>
      !query ||
      `${formatDate(meta.started_unix)} ${meta.preview}`
        .toLowerCase()
        .includes(query),
  );
  historyEmpty.classList.toggle("hidden", filtered.length > 0);
  historyEmpty.querySelector("h3")!.textContent = query
    ? "No matching conversations."
    : "A space for your conversations.";
  historyEmpty.querySelector("p")!.textContent = query
    ? "Try another date or a word from the conversation preview."
    : "End a coaching session to save its audio, transcript, and suggestions here.";
  document
    .getElementById("history-new-call")!
    .classList.toggle("hidden", !!query);
  historyWrap.classList.toggle("hidden", filtered.length === 0);
  historyRows.innerHTML = filtered.map((meta) => rowHtml(meta)).join("");
  setLiveUi();
}

function prospectBrief() {
  const brief = {
    name: prospectName.value.trim(),
    objective: callObjective.value,
    notes: prospectNotes.value.trim(),
    topic: meetingTopic.value.trim(),
  };
  return brief;
}

function briefForCoach() {
  const brief = prospectBrief();
  const parts = [
    brief.name && `Prospect/business: ${brief.name}`,
    `Goal: ${brief.objective}`,
    brief.objective === "General meeting" &&
      brief.topic &&
      `Meeting topic: ${brief.topic}`,
    brief.notes && `Context: ${brief.notes}`,
    brief.objective === "General meeting"
      ? "This is a general (non-sales) meeting. Coach only if the caller asks; otherwise help with clarity, note-taking, and follow-ups."
      : "Suggest questions and lines that move this sales call forward.",
  ].filter(Boolean);
  return parts.join(" | ");
}

function syncMeetingTopicField() {
  const isGeneral = callObjective.value === "General meeting";
  meetingTopicField.classList.toggle("hidden", !isGeneral);
}

function saveBrief() {
  localStorage.setItem(PROSPECT_KEY, JSON.stringify(prospectBrief()));
}

async function persist() {
  const k = keys();
  await invoke("save_config", k);
}

async function refreshCalls() {
  calls = await invoke<CallMeta[]>("list_calls");
  render();
}

async function startSession() {
  if (!dgKey.value.trim()) {
    openSetup("connections");
    dgKey.focus();
    setStatus("Add a captions key to start coaching.");
    return;
  }
  if (openrouterKey.value.trim() && !openrouterModel.value.trim()) {
    openSetup("connections");
    openrouterModel.focus();
    setStatus("Enter an OpenRouter model ID to use reply suggestions.");
    return;
  }
  startCall.disabled = true;
  try {
    const autoSuggest = autoAnswer.checked;
    localStorage.setItem(AUTO_KEY, autoSuggest ? "1" : "0");
    localStorage.setItem(
      OVERLAY_PREFS,
      JSON.stringify({ autoAnswer: autoSuggest }),
    );
    const k = keys();
    await invoke("save_config", k);
    await invoke("start_listening", {
      deepgramKey: k.deepgramKey,
      openaiKey: k.openaiKey,
      openaiModel: k.openaiModel,
      googleApiKey: k.googleApiKey,
      googleModel: k.googleModel,
      openrouterKey: k.openrouterKey,
      openrouterModel: k.openrouterModel,
      aiProvider: k.aiProvider,
      autoAnswer: autoSuggest,
      prospectBrief: briefForCoach(),
    });
    live = true;
    justFinished = false;
    document.getElementById("live-feed")!.innerHTML =
      '<p class="help">Captions appear as you and the prospect speak.</p>';
    document.getElementById("live-answer")!.textContent =
      "Your coach will suggest a line after the prospect speaks.";
    document.getElementById("live-coach-state")!.textContent =
      "Waiting for the prospect";
    document.getElementById("live-intent")!.textContent = "Listening";
    document.getElementById("live-goal")!.textContent =
      "Listen first. Keep the conversation natural.";
    setStatus("Connecting captions…", true);
    render();
  } catch (error) {
    showError(error);
  } finally {
    startCall.disabled = false;
  }
}

function clockOf(ms: number) {
  return new Date(ms).toLocaleTimeString([], {
    hour: "numeric",
    minute: "2-digit",
    second: "2-digit",
  });
}

function renderEntries(detail: CallDetail) {
  if (!detail.entries.length) {
    transcriptBody.innerHTML = `<pre class="transcript-raw">${escapeHtml(
      detail.transcript || "No transcript saved for this call.",
    )}</pre>`;
    return;
  }
  transcriptBody.innerHTML = detail.entries
    .map((entry) => {
      const who =
        entry.role === "you"
          ? "You"
          : entry.role === "coach"
            ? "Ghost Coach"
            : "Prospect";
      const intent = entry.intent ? ` · ${escapeHtml(entry.intent)}` : "";
      return `<article class="line ${entry.role}">
        <span class="who">${who}${intent}<span class="time">${clockOf(entry.at_unix_ms)}</span></span>
        <span>${escapeHtml(entry.text)}</span>
      </article>`;
    })
    .join("");
}

async function showCall(id: string, from: View) {
  selectedCall = id;
  backTo = from;

  const detail = await invoke<CallDetail>("load_call", { id });
  const suggestions = detail.entries.filter(
    (entry) => entry.role === "coach",
  ).length;
  transcriptTitle.textContent = callName(detail.meta);
  transcriptSub.textContent = `${formatDate(detail.meta.started_unix)} · ${formatDuration(detail.meta.duration_secs)} · ${detail.meta.line_count} spoken lines · ${suggestions} AI suggestions`;

  fillBriefFields(detail.brief);
  renderEntries(detail);
  showView("transcript");
}

function fillBriefFields(brief: CallDetail["brief"]) {
  briefAbout.value = brief.about || "";
  briefPoints.value = (brief.key_points || []).join("\n");
  briefNext.value = brief.next_step || "";
  briefNote.value = brief.user_note || "";
  briefSaved.textContent = "";
  briefCard.classList.toggle("hidden", false);
}

function currentBrief() {
  return {
    about: briefAbout.value.trim(),
    key_points: briefPoints.value
      .split("\n")
      .map((line) => line.trim())
      .filter(Boolean),
    next_step: briefNext.value.trim(),
    user_note: briefNote.value.trim(),
    generated: true,
  };
}

async function savePostCallBrief() {
  if (!selectedCall) {
    return;
  }
  try {
    await invoke("save_call_brief", { id: selectedCall, brief: currentBrief() });
    briefSaved.textContent = "Brief saved ✓";
  } catch (error) {
    briefSaved.textContent = "";
    showError(error);
  }
}

async function regeneratePostCallBrief() {
  if (!selectedCall) {
    return;
  }
  const button = document.querySelector("#regen-brief") as HTMLButtonElement;
  button.disabled = true;
  briefStatus.textContent = "Reading the transcript and writing the brief…";
  briefStatus.classList.remove("hidden");
  try {
    await invoke("save_call_brief", {
      id: selectedCall,
      brief: currentBrief(),
    });
    await invoke("generate_call_recap", { id: selectedCall });
  } catch (error) {
    showError(error);
    briefStatus.textContent = "";
    briefStatus.classList.add("hidden");
  } finally {
    button.disabled = false;
  }
}

startCall.addEventListener("click", () => {
  startSession().catch(showError);
});
stopCall.addEventListener("click", async () => {
  try {
    await invoke("stop_listening");
  } catch (error) {
    showError(error);
  }
});
showOverlayBtn.addEventListener("click", async () => {
  try {
    await invoke("show_overlay");
  } catch (error) {
    showError(error);
  }
});

nav.sessions.addEventListener("click", () => showView("sessions"));
nav.history.addEventListener("click", () => showView("history"));
nav.settings.addEventListener("click", () => openSetup());
document.querySelectorAll<HTMLButtonElement>(".setup-tab").forEach((tab) => {
  tab.addEventListener("click", () => openSetup(tab.dataset.setup));
});
document
  .getElementById("check-captions")!
  .addEventListener("click", () => openSetup("connections"));
document
  .getElementById("check-coach")!
  .addEventListener("click", () => openSetup("connections"));
document
  .getElementById("check-audio")!
  .addEventListener("click", () => openSetup("audio"));
document
  .getElementById("history-new-call")!
  .addEventListener("click", () => showView("sessions"));
document.getElementById("review-latest")!.addEventListener("click", () => {
  if (calls[0]) showCall(calls[0].id, "sessions").catch(showError);
});
document.getElementById("clear-brief")!.addEventListener("click", () => {
  [prospectName, prospectNotes, meetingTopic].forEach((input) => {
    input.value = "";
  });
  [callObjective].forEach((select) => {
    select.selectedIndex = 0;
  });
  syncMeetingTopicField();
  saveBrief();
  justFinished = false;
  setLiveUi();
  prospectName.focus();
});
document
  .getElementById("live-generate")!
  .addEventListener("click", async () => {
    const button = document.getElementById(
      "live-generate",
    ) as HTMLButtonElement;
    button.disabled = true;
    document.getElementById("live-coach-state")!.textContent = "Thinking…";
    try {
      await invoke("generate_answer");
    } catch (error) {
      document.getElementById("live-coach-state")!.textContent =
        "Coach unavailable";
      showError(error);
    } finally {
      button.disabled = false;
    }
  });
document
  .querySelector("#back")
  ?.addEventListener("click", () => showView(backTo));

historyRows.addEventListener("click", (event) => {
  const row = (event.target as HTMLElement).closest(
    "tr[data-call]",
  ) as HTMLElement | null;
  if (row?.dataset.call) {
    showCall(row.dataset.call, "history").catch(showError);
  }
});

function keyboardRow(event: KeyboardEvent) {
  if (event.key !== "Enter" && event.key !== " ") return;
  const row = (event.target as HTMLElement).closest(
    "tr[data-call]",
  ) as HTMLElement | null;
  if (row?.dataset.call) {
    event.preventDefault();
    showCall(
      row.dataset.call,
      row.parentElement?.parentElement?.parentElement?.id === "history-wrap"
        ? "history"
        : "sessions",
    ).catch(showError);
  }
}
historyRows.addEventListener("keydown", keyboardRow);

historySearch.addEventListener("input", render);
[prospectName, callObjective, prospectNotes, meetingTopic].forEach((el) => {
  el?.addEventListener("input", saveBrief);
  el?.addEventListener("change", saveBrief);
});
callObjective.addEventListener("change", syncMeetingTopicField);
[briefAbout, briefPoints, briefNext, briefNote].forEach((el) => {
  el.addEventListener("input", () => {
    briefSaved.textContent = "";
  });
});
document
  .querySelector("#save-brief")
  ?.addEventListener("click", () => savePostCallBrief().catch(showError));
document
  .querySelector("#regen-brief")
  ?.addEventListener("click", () => regeneratePostCallBrief().catch(showError));
[dgKey, openrouterKey, openrouterModel].forEach((field) => {
  field.addEventListener("input", () => {
    updateConnectionStatus();
    persist().catch(showError);
  });
});
autoAnswer.addEventListener("change", async () => {
  localStorage.setItem(AUTO_KEY, autoAnswer.checked ? "1" : "0");
  const prev = (() => {
    try {
      return JSON.parse(localStorage.getItem(OVERLAY_PREFS) || "{}");
    } catch {
      return {};
    }
  })();
  localStorage.setItem(
    OVERLAY_PREFS,
    JSON.stringify({ ...prev, autoAnswer: autoAnswer.checked }),
  );
  render();
  try {
    await invoke("set_auto_answer", { enabled: autoAnswer.checked });
  } catch {
    /* only applies while a session runs */
  }
});

[ovTheme, ovFont].forEach((el) => {
  el.addEventListener("input", () => {
    syncOverlayLabels();
    persist().catch(showError);
    pushOverlayPrefs().catch(showError);
  });
  el.addEventListener("change", () => {
    syncOverlayLabels();
    persist().catch(showError);
    pushOverlayPrefs().catch(showError);
  });
});

document
  .querySelector("#preview-overlay")
  ?.addEventListener("click", async () => {
    try {
      await pushOverlayPrefs();
      await invoke("show_overlay");
    } catch (error) {
      showError(error);
    }
  });

[
  document.querySelector("#open-folder"),
  document.querySelector("#open-folder-2"),
].forEach((btn) => {
  btn?.addEventListener("click", async () => {
    try {
      await invoke("open_calls_folder");
    } catch (error) {
      showError(error);
    }
  });
});

document.querySelector("#play-audio")?.addEventListener("click", async () => {
  if (!selectedCall) {
    return;
  }
  try {
    await invoke("open_call_audio", { id: selectedCall });
  } catch (error) {
    showError(error);
  }
});

document
  .querySelector("#copy-transcript")
  ?.addEventListener("click", async () => {
    await navigator.clipboard.writeText(transcriptBody.innerText || "");
    setStatus("Transcript copied.");
  });

document.querySelector("#test-mic")?.addEventListener("click", () => {
  listenTarget = "mic";
  audioHint.textContent = "Speak now. The microphone meter should move.";
  invoke("restart_meter_preview").catch(showError);
});
document.querySelector("#test-sys")?.addEventListener("click", () => {
  listenTarget = "sys";
  audioHint.textContent =
    "Play audio on this computer. The system meter should move.";
  invoke("restart_meter_preview").catch(showError);
});

function push(hist: number[], value: number) {
  hist.shift();
  hist.push(Math.max(0.04, Math.min(1, value * 10)));
}

function drawWave(canvas: HTMLCanvasElement, hist: number[]) {
  const ratio = window.devicePixelRatio || 1;
  const width = canvas.clientWidth || 180;
  const height = 36;
  canvas.width = Math.floor(width * ratio);
  canvas.height = Math.floor(height * ratio);
  const ctx = canvas.getContext("2d");
  if (!ctx) {
    return;
  }
  ctx.setTransform(ratio, 0, 0, ratio, 0, 0);
  ctx.clearRect(0, 0, width, height);
  const gap = 2;
  const bar = Math.max(2, (width - gap * (hist.length - 1)) / hist.length);
  hist.forEach((amp, i) => {
    const h = Math.max(3, amp * height * 0.92);
    ctx.fillStyle = "#16a34a";
    ctx.globalAlpha = 0.25 + amp * 0.75;
    ctx.fillRect(i * (bar + gap), height - h, bar, h);
  });
  ctx.globalAlpha = 1;
}

function paintMeters(you: number, them: number) {
  micDetected ||= you > 0.03;
  systemDetected ||= them > 0.03;
  updateReadiness();
  push(youHist, you);
  push(themHist, them);
  youMeter.classList.toggle("hot", you > 0.03);
  themMeter.classList.toggle("hot", them > 0.03);
  youLevel.textContent = you > 0.03 ? "Good" : "Quiet";
  themLevel.textContent = them > 0.03 ? "Good" : "Quiet";
  drawWave(youWave, youHist);
  drawWave(themWave, themHist);
  if (listenTarget === "mic" && you > 0.03) {
    audioHint.textContent = "Microphone is picking you up.";
  }
  if (listenTarget === "sys" && them > 0.03) {
    audioHint.textContent = "System audio is picking up the call.";
  }
}

async function initTauri() {
  while (!(window as any).__TAURI_INTERNALS__) {
    await new Promise((resolve) => setTimeout(resolve, 50));
  }

  await listen<{ text: string; live: boolean }>("status", (event) => {
    setStatus(event.payload.text, event.payload.live);
    if (event.payload.text.startsWith("Coach:")) {
      document.getElementById("live-coach-state")!.textContent =
        "Coach unavailable · try again";
    }
    const ended = live && !event.payload.live;
    live = event.payload.live;
    if (ended && /Session ended|Call saved/.test(event.payload.text))
      justFinished = true;
    setLiveUi();
    if (!live) {
      refreshCalls().catch(() => undefined);
      // The native recorder finishes writing session.json just after it emits
      // the stopped status. Refresh once more after that write is complete.
      window.setTimeout(() => refreshCalls().catch(() => undefined), 900);
    } else {
      render();
    }
  });
  await listen("calls-updated", () => {
    refreshCalls().catch(() => undefined);
    window.setTimeout(() => refreshCalls().catch(() => undefined), 250);
  });
  await listen<[string, string | null]>("brief-updated", (event) => {
    const [id, error] = event.payload;
    briefStatus.classList.add("hidden");
    if (error) {
      briefStatus.textContent = error;
      briefStatus.classList.remove("hidden");
      return;
    }
    if (id !== selectedCall) {
      return;
    }
    invoke<CallDetail>("load_call", { id })
      .then((detail) => {
        fillBriefFields(detail.brief);
        setStatus("Post-call brief ready.");
      })
      .catch(() => undefined);
  });
  await listen<{ you: number; them: number }>("meters", (event) => {
    paintMeters(event.payload.you, event.payload.them);
  });
  await listen<{ kind: string; text: string; speaker: string }>(
    "transcript",
    ({ payload }) => {
      const feed = document.getElementById("live-feed")!;
      const stick = feed.scrollHeight - feed.scrollTop - feed.clientHeight < 40;
      feed.querySelector(".help")?.remove();
      feed
        .querySelector(
          `[data-partial="${payload.speaker === "you" ? "you" : "them"}"]`,
        )
        ?.remove();
      const line = document.createElement("p");
      line.className = `live-line ${payload.kind === "partial" ? "partial" : ""}`;
      if (payload.kind === "partial")
        line.dataset.partial = payload.speaker === "you" ? "you" : "them";
      const label = document.createElement("small");
      label.textContent = payload.speaker === "you" ? "YOU" : "PROSPECT";
      line.append(label, document.createTextNode(payload.text));
      feed.appendChild(line);
      while (feed.children.length > 80) feed.firstElementChild?.remove();
      if (stick) feed.scrollTop = feed.scrollHeight;
    },
  );
  await listen<{ answer: string; why: string; tag: string }>(
    "coach",
    ({ payload }) => {
      const state = document.getElementById("live-coach-state")!;
      if (payload.tag === "CAPTION") {
        const k = keys();
        const coach = !!k.openrouterKey && !!k.openrouterModel;
        state.textContent = !coach
          ? "Captions only · coach key needed"
          : autoAnswer.checked
            ? "Thinking…"
            : "Suggestions on demand";
      } else if (payload.tag === "WAIT") {
        state.textContent = "Waiting for the prospect";
      } else if (payload.tag === "ERROR") {
        state.textContent = "Coach unavailable · captions continue";
        document.getElementById("live-goal")!.textContent =
          "Use the floating coach for a backup question, or try again.";
      } else if (payload.answer) {
        document.getElementById("live-answer")!.textContent = payload.answer;
        document.getElementById("live-intent")!.textContent =
          payload.tag || "Next line";
        document.getElementById("live-goal")!.textContent = payload.why
          ? `Aim: ${payload.why}`
          : "Keep the conversation natural.";
        state.textContent = "Suggestion ready";
      }
    },
  );
  await listen<string>("audio-preview-error", (event) => {
    audioHint.textContent =
      event.payload +
      ". Install the Linux audio tools and check your default devices.";
    setStatus("Audio device unavailable.");
  });

  const saved = await invoke<{
    deepgram_key: string;
    openai_key: string;
    openai_model: string;
    google_api_key: string;
    google_model: string;
    openrouter_key: string;
    openrouter_model: string;
    ai_provider: string;
    overlay?: {
      theme?: string;
      font_scale?: number;
    };
  }>("load_config");
  dgKey.value = saved.deepgram_key ?? "";
  previousConnections = {
    openaiKey: saved.openai_key ?? "",
    openaiModel: saved.openai_model ?? "",
    googleApiKey: saved.google_api_key ?? "",
    googleModel: saved.google_model ?? "",
  };
  openrouterKey.value = saved.openrouter_key ?? "";
  openrouterModel.value = saved.openrouter_model ?? "";

  const storedAutoAnswer = localStorage.getItem(AUTO_KEY);
  autoAnswer.checked =
    storedAutoAnswer === null ? true : storedAutoAnswer === "1";
  try {
    const brief = JSON.parse(localStorage.getItem(PROSPECT_KEY) || "{}");
    prospectName.value = brief.name || "";

    callObjective.value = brief.objective || callObjective.value;
    meetingTopic.value = brief.topic || "";
    prospectNotes.value = brief.notes || "";
  } catch {
    /* ignore invalid local brief */
  }
  syncMeetingTopicField();
  saveBrief();
  applySavedOverlay(saved.overlay ?? {});
  updateConnectionStatus();
  await persist();
  paintMeters(0, 0);
  await pushOverlayPrefs().catch(() => undefined);
  await refreshCalls();
}

render();
initTauri().catch(showError);
