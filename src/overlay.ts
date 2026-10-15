import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const timer = document.querySelector("#timer") as HTMLElement;
const liveBadge = document.querySelector("#live-badge") as HTMLElement;
const health = document.querySelector("#health") as HTMLElement;
const intentBadge = document.querySelector("#intent-badge") as HTMLElement;
const trigger = document.querySelector("#trigger") as HTMLElement;
const triggerText = document.querySelector("#trigger-text") as HTMLElement;
const answerEl = document.querySelector("#answer") as HTMLElement;
const goalEl = document.querySelector("#goal") as HTMLElement;
const coachState = document.querySelector("#coach-state") as HTMLElement;
const coachStateText = document.querySelector(
  "#coach-state-text",
) as HTMLElement;
const needEl = document.querySelector("#need") as HTMLElement;
const feed = document.querySelector("#feed") as HTMLElement;
const idle = document.querySelector("#idle") as HTMLElement;
const autoAnswer = document.querySelector("#auto-answer") as HTMLInputElement;
const generateBtn = document.querySelector("#generate") as HTMLButtonElement;
const intentInput = document.querySelector("#intent-input") as HTMLTextAreaElement;
const polishBtn = document.querySelector("#polish") as HTMLButtonElement;
const polishStatus = document.querySelector("#polish-status") as HTMLElement;

type CoachState = "idle" | "listening" | "thinking" | "ready" | "error";

type OverlayLook = {
  theme: string;
  font_scale: number;
};

function applyLook(prefs: Partial<OverlayLook>) {
  const theme = prefs.theme === "dark" ? "dark" : "light";
  document.documentElement.dataset.theme = theme;
  document.documentElement.style.setProperty(
    "--font-scale",
    String(prefs.font_scale ?? 1),
  );
}

const IDLE_ANSWER =
  "Your next line appears here as soon as they finish talking.";

// Only used when a coach request fails. A broken provider should degrade into a
// usable question rather than an error message in the middle of a call — and it
// must never pre-empt the real answer, so nothing shows until the request fails.
const FALLBACKS = [
  {
    match: /price|cost|budget|expensive|afford/i,
    line: "What would make this feel like a worthwhile investment?",
    goal: "understand value and budget",
  },
  {
    match: /think|later|timing|busy|not now/i,
    line: "What would you need to feel confident moving forward?",
    goal: "uncover the real blocker",
  },
  {
    match: /website|site|page|online/i,
    line: "How is your current website helping you generate work today?",
    goal: "connect the current setup to business results",
  },
  {
    match: /lead|call|customer|booking|referral/i,
    line: "How are you handling that today, and where does it break down?",
    goal: "find the operational pain",
  },
  {
    match: /who|owner|partner|decision/i,
    line: "Who else should be involved before you decide?",
    goal: "confirm the decision process",
  },
];

const COACH_LABELS: Record<CoachState, string> = {
  idle: "Idle",
  listening: "Listening",
  thinking: "Thinking",
  ready: "Ready",
  error: "Needs attention",
};

let lastProspect = "";
let startedAt = 0;
let tick: number | undefined;

function setCoachState(state: CoachState) {
  coachState.dataset.state = state;
  coachStateText.textContent = COACH_LABELS[state];
  coachState.title = `Coach: ${COACH_LABELS[state]}`;
}

function escapeText(value: string) {
  return value
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

function formatElapsed(ms: number) {
  const total = Math.floor(ms / 1000);
  const m = String(Math.floor(total / 60)).padStart(2, "0");
  const s = String(total % 60).padStart(2, "0");
  return `${m}:${s}`;
}

function startTimer() {
  startedAt = Date.now();
  window.clearInterval(tick);
  tick = window.setInterval(() => {
    timer.textContent = formatElapsed(Date.now() - startedAt);
  }, 250);
  liveBadge.textContent = "LIVE";
  liveBadge.classList.remove("off");
}

function reset() {
  window.clearInterval(tick);
  tick = undefined;
  timer.textContent = "00:00";
  liveBadge.textContent = "READY";
  liveBadge.classList.add("off");
  intentBadge.textContent = "Listening";
  setCoachState("idle");
  lastProspect = "";
  trigger.classList.add("hidden");
  triggerText.textContent = "";
  answerEl.textContent = IDLE_ANSWER;
  answerEl.classList.add("placeholder");
  goalEl.textContent = "Goal: understand the prospect’s current situation.";
  needEl.textContent = "";
  feed.replaceChildren(idle);
  health.textContent = "Audio quiet";
}

/** The prospect line that prompted the current suggestion — the "why" of the hero line. */
function showTrigger(text: string) {
  const trimmed = text.trim();
  if (!trimmed) {
    trigger.classList.add("hidden");
    return;
  }
  triggerText.textContent = trimmed;
  trigger.classList.remove("hidden");
}

function atBottom(el: HTMLElement) {
  return el.scrollHeight - el.scrollTop - el.clientHeight < 28;
}

function upsertLine(speaker: "you" | "them", text: string, final: boolean) {
  // Only follow the conversation when the reader is already at the newest line —
  // otherwise scrolling back to re-read what the prospect said is impossible.
  const stick = atBottom(feed);
  idle.remove();
  const label = speaker === "you" ? "Me" : "Prospect";
  const partialId = `partial-${speaker}`;
  document.getElementById(partialId)?.remove();
  const row = document.createElement("p");
  row.className = `msg ${speaker}${final ? "" : " partial"}`;
  if (!final) {
    row.id = partialId;
  }
  row.innerHTML = `<span class="who">${label}</span><span class="txt">${escapeText(text)}</span>`;
  feed.appendChild(row);
  while (feed.querySelectorAll(".msg").length > 40) {
    feed.querySelector(".msg")?.remove();
  }
  if (stick) {
    feed.scrollTop = feed.scrollHeight;
  }
}

function setAnswer(text: string) {
  answerEl.textContent = text || IDLE_ANSWER;
  answerEl.classList.toggle("placeholder", !text);
}

function syncGenerateBtn() {
  generateBtn.classList.toggle("hidden", autoAnswer.checked);
}

function applyAuto(enabled: boolean) {
  autoAnswer.checked = enabled;
  let prev: Record<string, unknown> = {};
  try {
    prev = JSON.parse(localStorage.getItem("sonic-overlay-prefs") || "{}");
  } catch {
    prev = {};
  }
  localStorage.setItem(
    "sonic-overlay-prefs",
    JSON.stringify({ ...prev, autoAnswer: enabled }),
  );
  syncGenerateBtn();
}

autoAnswer.addEventListener("change", async () => {
  applyAuto(autoAnswer.checked);
  try {
    await invoke("set_auto_answer", { enabled: autoAnswer.checked });
  } catch {
    /* session not running */
  }
});

generateBtn.addEventListener("click", async () => {
  generateBtn.disabled = true;
  setCoachState("thinking");
  needEl.textContent = "Drafting the next line…";
  try {
    await invoke("generate_answer");
  } catch (error) {
    setCoachState("error");
    needEl.textContent = error instanceof Error ? error.message : String(error);
  } finally {
    generateBtn.disabled = false;
  }
});

document.querySelector("#collapse")?.addEventListener("click", () => {
  invoke("hide_overlay").catch(() => undefined);
});
document.querySelector("#close")?.addEventListener("click", () => {
  invoke("stop_listening").catch(() => undefined);
});

/** The polished line replaces the hero answer and must survive the next
    coach tick for a few seconds, otherwise typing feedback gets wiped
    before the user has read it. */
let polishHoldUntil = 0;

async function polishIntent() {
  const text = intentInput.value.trim();
  if (!text) {
    polishStatus.textContent = "Type your intention first.";
    polishStatus.classList.add("error");
    return;
  }
  polishBtn.disabled = true;
  intentInput.disabled = true;
  polishStatus.classList.remove("error");
  polishStatus.textContent = "Translating…";
  setCoachState("thinking");
  try {
    const line = await invoke<string>("polish_line", { text });
    setAnswer(line);
    answerEl.classList.remove("placeholder");
    intentBadge.textContent = "Polished";
    goalEl.textContent = "Goal: say your line — polished for you.";
    needEl.textContent = "";
    polishStatus.textContent = "";
    intentInput.value = "";
    setCoachState("ready");
    polishHoldUntil = Date.now() + 20_000;
  } catch (error) {
    polishStatus.textContent =
      error instanceof Error ? error.message : String(error);
    polishStatus.classList.add("error");
    setCoachState("error");
  } finally {
    polishBtn.disabled = false;
    intentInput.disabled = false;
    intentInput.focus();
  }
}

polishBtn.addEventListener("click", () => {
  void polishIntent();
});
intentInput.addEventListener("keydown", (event) => {
  if (event.key === "Enter" && !event.shiftKey) {
    event.preventDefault();
    void polishIntent();
  }
});

try {
  const prefs = JSON.parse(localStorage.getItem("sonic-overlay-prefs") || "{}");
  applyAuto(typeof prefs.autoAnswer === "boolean" ? prefs.autoAnswer : true);
} catch {
  applyAuto(true);
}

async function initTauri() {
  while (!(window as any).__TAURI_INTERNALS__) {
    await new Promise((resolve) => setTimeout(resolve, 50));
  }

  await listen<{ text: string; live: boolean }>("status", (event) => {
    if (event.payload.text.startsWith("Coach:")) {
      setCoachState("error");
      needEl.textContent =
        "Coach unavailable. Try another reply or continue the conversation.";
    }
    if (event.payload.live) {
      if (!tick) {
        startTimer();
      }
    } else {
      reset();
      if (event.payload.text.startsWith("Session failed:")) {
        liveBadge.textContent = "NOT RECORDING";
        setCoachState("error");
        setAnswer(
          "The session could not start. Check Setup in the main window, then try again.",
        );
        needEl.textContent = event.payload.text.replace(
          /^Session failed:\s*/,
          "",
        );
        goalEl.textContent =
          "This coach stays visible so you can see what needs fixing.";
      }
    }
  });

  await listen<{ auto_answer: boolean; coach_ready: boolean }>(
    "session-ui",
    (event) => {
      applyAuto(event.payload.auto_answer);
      if (!event.payload.coach_ready) {
        intentBadge.textContent = "Captions only";
        needEl.textContent =
          "Add a coach API key in Settings to suggest the next line.";
        setCoachState("error");
      } else {
        needEl.textContent =
          "Listening — a next line will appear after they speak.";
        setCoachState("listening");
      }
    },
  );

  await listen<{ kind: string; text: string; speaker: string }>(
    "transcript",
    (event) => {
      const speaker = event.payload.speaker === "you" ? "you" : "them";
      const final = event.payload.kind === "final";
      upsertLine(speaker, event.payload.text, final);
      if (speaker === "them" && final) {
        lastProspect = event.payload.text;
        showTrigger(event.payload.text);
        setCoachState("listening");
      }
    },
  );

  await listen<{ question: string; answer: string; why: string; tag: string }>(
    "coach",
    (event) => {
      const { answer, why, tag } = event.payload;
      if (tag === "CAPTION") {
        setCoachState("thinking");
        needEl.textContent = why || "Drafting the next line…";
        return;
      }
      if (tag === "WAIT") {
        setCoachState("listening");
        needEl.textContent =
          why || "Nothing to answer yet. Wait for them to speak.";
        return;
      }
      if (tag === "ERROR") {
        intentBadge.textContent = "Fallback";
        setCoachState("error");
        const candidate =
          FALLBACKS.find((item) => item.match.test(lastProspect)) ||
          FALLBACKS[3];
        setAnswer(candidate.line);
        goalEl.textContent = `Goal: ${candidate.goal}`;
        needEl.textContent = "Coach unavailable — using a backup question.";
        return;
      }
      if (tag) {
        intentBadge.textContent = tag;
      }
      if (answer) {
        // Let a just-polished line stay readable before live coaching takes over.
        if (Date.now() < polishHoldUntil) {
          return;
        }
        setAnswer(answer);
        setCoachState("ready");
        goalEl.textContent = why
          ? `Goal: ${why}`
          : "Goal: move the conversation forward.";
        needEl.textContent = "";
      } else if (why) {
        needEl.textContent = why;
      }
    },
  );

  await listen<{ you: number; them: number }>("meters", (event) => {
    const mic = event.payload.you > 0.03;
    const sys = event.payload.them > 0.03;
    health.textContent = sys
      ? "System audio good"
      : mic
        ? "Microphone good"
        : "Audio quiet";
  });

  await listen<OverlayLook>("overlay-prefs", (event) => {
    applyLook(event.payload);
  });

  try {
    const saved = await invoke<{ overlay?: OverlayLook }>("load_config");
    if (saved.overlay) {
      applyLook(saved.overlay);
    }
  } catch {
    /* defaults are fine */
  }
}

initTauri();
