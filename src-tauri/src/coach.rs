use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

fn http() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(6))
            .pool_max_idle_per_host(4)
            .tcp_nodelay(true)
            .build()
            .expect("http client")
    })
}

static LAST_GEMINI: Mutex<Option<String>> = Mutex::new(None);

#[derive(Clone, Debug)]
pub struct CoachSettings {
    pub provider: String,
    pub google_api_key: String,
    pub google_model: String,
    pub openai_key: String,
    pub openai_model: String,
    pub openrouter_key: String,
    pub openrouter_model: String,
}

#[derive(Clone, Debug, Default)]
pub struct CoachReply {
    pub question: String,
    pub answer: String,
    pub why: String,
    pub tag: String,
}

impl CoachSettings {
    pub fn enabled(&self) -> bool {
        match self.provider.as_str() {
            "openai" => !self.openai_key.is_empty(),
            "openrouter" => !self.openrouter_key.is_empty(),
            _ => !self.google_api_key.is_empty(),
        }
    }

    fn openai_model(&self) -> &str {
        if self.openai_model.trim().is_empty() {
            "gpt-4o-mini"
        } else {
            self.openai_model.trim()
        }
    }

    fn openrouter_model(&self) -> Result<String> {
        let model = self.openrouter_model.trim();
        if model.is_empty() {
            bail!("Enter an OpenRouter model ID in Connections.");
        }
        Ok(model.to_string())
    }

    fn gemini_models(&self) -> Vec<String> {
        let mut models = Vec::new();
        if let Ok(last) = LAST_GEMINI.lock() {
            if let Some(name) = last.clone() {
                models.push(name);
            }
        }
        for fast in [
            "gemini-2.5-flash-lite",
            "gemini-2.0-flash-lite",
            "gemini-2.0-flash",
        ] {
            if !models.iter().any(|m| m == fast) {
                models.push(fast.to_string());
            }
        }
        let preferred = self.google_model.trim();
        if !preferred.is_empty() && !models.iter().any(|m| m == preferred) {
            models.push(preferred.to_string());
        }
        for fallback in ["gemini-2.5-flash", "gemini-flash-latest"] {
            if !models.iter().any(|m| m == fallback) {
                models.push(fallback.to_string());
            }
        }
        models
    }
}

const SALES_SYSTEM_PROMPT: &str = "You are Ghost Coach, a silent live copilot for a consultant selling websites to local service businesses, especially plumbers. Return JSON only: {\"question\":\"short label for what they revealed\",\"answer\":\"the exact next sentence the salesperson should say aloud\",\"why\":\"max 6 words\",\"tag\":\"Rapport|Discovery|Pain|Impact|Vision|Offer|Objection|Close|Hold\"}. Rules: diagnose before prescribing; use the discovery checklist as a map, not a script; ask one question at a time; respond to the prospect's newest line before moving on; if they ask a direct question, answer it in the first sentence; sell outcomes such as more calls, better jobs, trust, and saved time rather than design features; handle objections with curiosity, never argue; ask for a specific next step once the problem and value are clear; never invent prices, rankings, case studies, guarantees, or results; never repeat a question or suggestion already in the transcript. The answer must be natural spoken English under 20 words.";

const GENERAL_SYSTEM_PROMPT: &str = "Silent meeting copilot. Return JSON only: {\"question\":\"short label\",\"answer\":\"the exact next sentence the user should say aloud\",\"why\":\"max 6 words\",\"tag\":\"Discovery|Impact|Offer|Objection|Next step|Hold\"}. Be concise, accurate, and useful. Coach only when helpful; otherwise help with clarity and the next action. Never invent facts, pricing, or results. Keep the answer under 20 words.";

fn is_sales_mode(brief: &str, open_questions: &[String]) -> bool {
    !open_questions.is_empty()
        || brief
            .to_ascii_lowercase()
            .contains("call type: plumbing sales call")
}

/// Fast deterministic read on the most common plumber objections. The model
/// still chooses the final wording; this keeps it from pitching past the issue.
fn objection_hint(prospect: &str) -> Option<&'static str> {
    let said = prospect.to_ascii_lowercase();
    let signals: &[(&[&str], &str)] = &[
        (
            &["already have a website", "already got a website", "have a site", "got a site"],
            "They mentioned an existing site. Ask whether it brings in calls and when it was last updated; do not pitch yet.",
        ),
        (
            &["word of mouth", "word-of-mouth", "referral", "referrals"],
            "They rely on referrals. Explore slow periods, after-hours calls, and the high-value jobs they want more of; do not attack referrals.",
        ),
        (
            &["too expensive", "too much money", "can't afford", "budget", "price", "pricing", "how much", "cost", "quote"],
            "Price or budget came up. Do not guess a number; ask which jobs or outcomes matter most, then frame the next step as a scoped proposal.",
        ),
        (
            &["too busy", "no time", "busy season", "slammed"],
            "They are time-poor. Emphasize a done-for-you process and minimal owner time; do not add pressure.",
        ),
        (
            &["my nephew", "my cousin", "my friend", "my brother", "my son", "someone built", "freelancer"],
            "Someone they know built the current site. Respect the work and ask what happens when it needs updates, tracking, or ongoing improvements.",
        ),
        (
            &["don't need more", "do not need more", "enough work", "too much work", "fully booked"],
            "They do not need more volume. Shift to better jobs, stronger trust, and fewer time-wasting enquiries.",
        ),
        (
            &["google", "seo", "ranking", "rank", "search"],
            "Search or SEO came up. Speak plainly about local visibility and conversion; never promise a ranking or lead count.",
        ),
        (
            &["bad experience", "tried before", "wasted money", "didn't work", "did not work", "scammed"],
            "They had a bad experience. Ask what went wrong and what would make this different; diagnose before defending yourself.",
        ),
        (
            &["send me", "email me", "send some info", "send information"],
            "They asked for information. Agree, confirm the one detail that matters most, and lock in a specific follow-up time.",
        ),
        (
            &["not interested", "not right now", "maybe later", "call me later"],
            "They are not ready. Ask what would need to change for a website to matter, or ask permission for a low-pressure follow-up.",
        ),
    ];
    signals
        .iter()
        .find(|(needles, _)| needles.iter().any(|needle| said.contains(needle)))
        .map(|(_, hint)| *hint)
}

fn user_prompt(
    prospect: &str,
    history: &[String],
    brief: &str,
    summary: &str,
    open_questions: &[String],
    recent_coach: &[String],
) -> String {
    let recent = history
        .iter()
        .rev()
        .take(8)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    let memory = if summary.is_empty() {
        "(nothing yet)".to_string()
    } else {
        summary.to_string()
    };
    let mut open = String::new();
    for (i, q) in open_questions.iter().enumerate().take(8) {
        open.push_str(&format!("{}. {q}\n", i + 1));
    }
    let open_block = if open.is_empty() {
        "(none - every playbook question has been answered)".to_string()
    } else {
        open
    };
    let coaching_read = if is_sales_mode(brief, open_questions) {
        objection_hint(prospect).unwrap_or(
            "No obvious objection. React to their newest line, then choose the highest-value open question.",
        )
    } else {
        "No special objection detected."
    };
    let prior = if recent_coach.is_empty() {
        "(none yet)".to_string()
    } else {
        recent_coach
            .iter()
            .rev()
            .take(5)
            .rev()
            .map(|line| format!("- {line}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "BRIEF:\n{brief}\n\nCONVERSATION MEMORY:\n{memory}\n\nRECENT LINES:\n{recent}\n\nTHEY JUST SAID:\n{prospect}\n\nDISCOVERY CHECKLIST STILL OPEN (choose only if it is the best next move):\n{open_block}\n\nCOACHING READ:\n{coaching_read}\n\nYOUR RECENT SUGGESTIONS (do not repeat):\n{prior}\n\nTASK:\nWrite the single best next sentence for the salesperson. If they asked a direct question, answer it first. If they revealed a problem or objection, address that before moving on. Otherwise ask the highest-value open question. Do not mechanically march through the checklist. Use their exact words where useful. Make it sound like a confident human, not a script."
    )
}

pub async fn warmup(settings: &CoachSettings) {
    let _ = suggest(settings, "Hello", &[], "", "", &[], &[]).await;
}

const SUMMARY_PROMPT: &str = "Conversation memory keeper for a live sales call. You get the running memory so far plus the newest lines. JSON only: {\"summary\":\"3 sentences max: what the prospect has revealed and agreed so far\"}. Carry forward every concrete fact (numbers, names, timelines, pain points) and drop small talk. Never invent facts.";

fn summarize_user_prompt(current: &str, latest_lines: &[String]) -> String {
    let memory = if current.trim().is_empty() {
        "(none yet)".to_string()
    } else {
        current.trim().to_string()
    };
    let lines = latest_lines.join("\n");
    format!(
        "CURRENT MEMORY:\n{memory}\nNEWEST LINES:\n{lines}\n\nMerge the newest lines into the memory. Keep it under 90 words."
    )
}

fn parse_summary(raw: &str) -> Result<String> {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let parsed: Value = serde_json::from_str(trimmed)
        .map_err(|_| anyhow::anyhow!("summary response was not valid JSON"))?;
    Ok(parsed
        .get("summary")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .trim()
        .to_string())
}

/// Rewrite the running memory so far. Mechanical fallback merges the old
/// summary with the newest lines when no AI key is configured.
pub async fn summarize(settings: &CoachSettings, current: &str, latest_lines: &[String]) -> String {
    if settings.enabled() && !latest_lines.is_empty() {
        let result = match settings.provider.as_str() {
            "openai" => summarize_openai(settings, current, latest_lines).await,
            "openrouter" => summarize_openrouter(settings, current, latest_lines).await,
            _ => summarize_gemini(settings, current, latest_lines).await,
        };
        if let Ok(text) = result {
            if !text.trim().is_empty() {
                return text.trim().to_string();
            }
        }
    }
    summarize_offline(current, latest_lines)
}

fn summarize_offline(current: &str, latest_lines: &[String]) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }
    parts.extend(
        latest_lines
            .iter()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty()),
    );
    let merged = parts.join(" ");
    // Hard cap so the fallback cannot grow without bound.
    match merged.char_indices().nth(700) {
        Some((idx, _)) => merged[..idx].to_string(),
        None => merged,
    }
}

async fn summarize_openai(
    settings: &CoachSettings,
    current: &str,
    latest_lines: &[String],
) -> Result<String> {
    let body = json!({
        "model": settings.openai_model(),
        "temperature": 0.1,
        "max_tokens": 140,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": SUMMARY_PROMPT },
            { "role": "user", "content": summarize_user_prompt(current, latest_lines) }
        ]
    });
    let res = http()
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(&settings.openai_key)
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    let v: Value = res.json().await?;
    if !status.is_success() {
        bail!(
            "openai HTTP {status}: {}",
            v["error"]["message"].as_str().unwrap_or("request failed")
        );
    }
    let content = v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("{}");
    parse_summary(content)
}

async fn summarize_openrouter(
    settings: &CoachSettings,
    current: &str,
    latest_lines: &[String],
) -> Result<String> {
    let body = json!({
        "model": settings.openrouter_model()?,
        "temperature": 0.1,
        "max_tokens": 140,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": SUMMARY_PROMPT },
            { "role": "user", "content": summarize_user_prompt(current, latest_lines) }
        ]
    });
    let res = http()
        .post("https://openrouter.ai/api/v1/chat/completions")
        .timeout(Duration::from_secs(20))
        .bearer_auth(&settings.openrouter_key)
        .header("HTTP-Referer", "https://sonicsales.app")
        .header("X-Title", "Ghost Coach")
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    let v: Value = res.json().await?;
    if !status.is_success() {
        bail!(
            "openrouter HTTP {status}: {}",
            v["error"]["message"].as_str().unwrap_or("request failed")
        );
    }
    let content = v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("{}");
    parse_summary(content)
}

async fn summarize_gemini(
    settings: &CoachSettings,
    current: &str,
    latest_lines: &[String],
) -> Result<String> {
    let prompt = format!(
        "{SUMMARY_PROMPT}\n\n{}",
        summarize_user_prompt(current, latest_lines)
    );
    let body = json!({
        "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
        "generationConfig": {
            "temperature": 0.1,
            "maxOutputTokens": 140,
            "responseMimeType": "application/json"
        }
    });
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        settings.gemini_models()[0],
        settings.google_api_key
    );
    let res = http().post(&url).json(&body).send().await?;
    let status = res.status();
    let v: Value = res.json().await?;
    if !status.is_success() {
        bail!("gemini HTTP {status}");
    }
    let text = v["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap_or("{}");
    parse_summary(text)
}

const POLISH_PROMPT: &str = "Line polisher for live sales calls. Input is the speaker's intent, often Banglish (Bengali written in Latin script), sometimes mixed with English or Bengali script. Output ONE polished professional spoken English sentence that keeps the exact intent, tone-appropriate for a confident salesperson. Natural to say aloud, under 30 words. No explanations, no quotes. JSON only: {\"line\":\"...\"}. If the input is already clean English, just polish it lightly.";

fn polish_user_prompt(text: &str) -> String {
    format!(
        "INTENT:\n{}\n\nPolish this into one professional spoken English line.",
        text.trim()
    )
}

fn parse_line(raw: &str) -> Result<String> {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let parsed: Value = serde_json::from_str(trimmed)
        .map_err(|_| anyhow::anyhow!("polish response was not valid JSON"))?;
    Ok(parsed
        .get("line")
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .trim()
        .to_string())
}

async fn polish_openai(settings: &CoachSettings, text: &str) -> Result<String> {
    let body = json!({
        "model": settings.openai_model(),
        "temperature": 0.3,
        "max_tokens": 120,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": POLISH_PROMPT },
            { "role": "user", "content": polish_user_prompt(text) }
        ]
    });
    let res = http()
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(&settings.openai_key)
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    let v: Value = res.json().await?;
    if !status.is_success() {
        bail!(
            "openai HTTP {status}: {}",
            v["error"]["message"].as_str().unwrap_or("request failed")
        );
    }
    let content = v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("{}");
    parse_line(content)
}

async fn polish_openrouter(settings: &CoachSettings, text: &str) -> Result<String> {
    let model = settings.openrouter_model()?;
    let mut body = json!({
        "model": model,
        "temperature": 0.3,
        "max_tokens": 500,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": POLISH_PROMPT },
            { "role": "user", "content": polish_user_prompt(text) }
        ]
    });
    if model.contains("gpt-6") {
        body["reasoning"] = json!({ "effort": "none" });
        body["max_completion_tokens"] = json!(500);
    }
    let res = http()
        .post("https://openrouter.ai/api/v1/chat/completions")
        .timeout(Duration::from_secs(20))
        .bearer_auth(&settings.openrouter_key)
        .header("HTTP-Referer", "https://sonicsales.app")
        .header("X-Title", "Ghost Coach")
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    let v: Value = res.json().await?;
    if !status.is_success() {
        bail!(
            "openrouter HTTP {status}: {}",
            v["error"]["message"].as_str().unwrap_or("request failed")
        );
    }
    let content = v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("{}");
    parse_line(content)
}

async fn polish_gemini(settings: &CoachSettings, text: &str) -> Result<String> {
    let prompt = format!("{POLISH_PROMPT}\n\n{}", polish_user_prompt(text));
    let body = json!({
        "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
        "generationConfig": {
            "temperature": 0.3,
            "maxOutputTokens": 120,
            "responseMimeType": "application/json"
        }
    });
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        settings.gemini_models()[0],
        settings.google_api_key
    );
    let res = http().post(&url).json(&body).send().await?;
    let status = res.status();
    let v: Value = res.json().await?;
    if !status.is_success() {
        bail!("gemini HTTP {status}");
    }
    let text = v["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .unwrap_or("{}");
    parse_line(text)
}

/// Banglish -> professional spoken English. Uses the live session's coach
/// settings when one is running, otherwise the saved config keys.
pub async fn polish(settings: &CoachSettings, text: &str) -> Result<String> {
    let result = match settings.provider.as_str() {
        "openai" => polish_openai(settings, text).await,
        "openrouter" => polish_openrouter(settings, text).await,
        _ => polish_gemini(settings, text).await,
    };
    let line = result?;
    if line.is_empty() {
        bail!("The translator returned an empty line.");
    }
    Ok(line)
}

const RECAP_PROMPT: &str = "Meeting recap writer. JSON only: {\"about\":\"one sentence: what this meeting was about\",\"key_points\":[\"up to 5 short points, each under 18 words\"],\"next_step\":\"the single clearest follow-up action, or 'No clear next step was agreed'\"}. Use only what was said; never invent facts.";

fn recap_user_prompt(history: &[String]) -> String {
    let lines = history
        .iter()
        .map(|line| line.replace("You: ", "Caller: "))
        .collect::<Vec<_>>()
        .join("\n");
    format!("CONVERSATION TRANSCRIPT:\n{lines}\n\nWrite the recap from the caller's point of view.")
}

#[derive(Clone, Debug, Default)]
pub struct RecapOutput {
    pub about: String,
    pub key_points: Vec<String>,
    pub next_step: String,
}

impl RecapOutput {
    fn is_empty(&self) -> bool {
        self.about.is_empty() && self.key_points.is_empty() && self.next_step.is_empty()
    }
}

pub async fn recap(settings: &CoachSettings, history: &[String]) -> Result<RecapOutput> {
    if settings.enabled() {
        let result = match settings.provider.as_str() {
            "openai" => recap_openai(settings, history).await,
            "openrouter" => recap_openrouter(settings, history).await,
            _ => recap_gemini(settings, history).await,
        };
        if let Ok(out) = result {
            return Ok(out);
        }
    }
    Ok(recap_offline(history))
}

/// Fallback when no AI key is set: a mechanical recap from the transcript.
fn recap_offline(history: &[String]) -> RecapOutput {
    let spoken: Vec<String> = history
        .iter()
        .filter(|line| !line.starts_with("Coach:"))
        .cloned()
        .collect();
    if spoken.is_empty() {
        return RecapOutput {
            about: "No spoken lines were captured for this session.".into(),
            key_points: Vec::new(),
            next_step: String::new(),
        };
    }
    let them_lines = spoken
        .iter()
        .filter(|line| line.starts_with("Prospect:"))
        .count();
    let you_lines = spoken.len() - them_lines;
    let duration_hint = if spoken.len() > 20 { "longer" } else { "short" };
    RecapOutput {
        about: format!(
            "A {duration_hint} conversation with {you_lines} lines from you and {them_lines} from them."
        ),
        key_points: spoken.iter().rev().take(3).rev().cloned().collect(),
        next_step: "Add an AI key in Setup for a smart recap and next step.".into(),
    }
}

fn parse_recap(raw: &str) -> Result<RecapOutput> {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let parsed: Value = serde_json::from_str(trimmed)
        .map_err(|_| anyhow::anyhow!("The recap returned an invalid format."))?;
    let about = parsed
        .get("about")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let key_points = parsed
        .get("key_points")
        .and_then(|x| x.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(|item| item.trim().to_string())
                .filter(|item| !item.is_empty())
                .take(5)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let next_step = parsed
        .get("next_step")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let out = RecapOutput {
        about,
        key_points,
        next_step,
    };
    if out.is_empty() {
        bail!("The recap was empty.");
    }
    Ok(out)
}

async fn recap_openai(settings: &CoachSettings, history: &[String]) -> Result<RecapOutput> {
    let body = json!({
        "model": settings.openai_model(),
        "temperature": 0.2,
        "max_tokens": 400,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": RECAP_PROMPT },
            { "role": "user", "content": recap_user_prompt(history) }
        ]
    });
    let res = http()
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(&settings.openai_key)
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    let v: Value = res.json().await?;
    if !status.is_success() {
        bail!(
            "openai HTTP {status}: {}",
            v["error"]["message"].as_str().unwrap_or("request failed")
        );
    }
    parse_recap(
        v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("{}"),
    )
}

async fn recap_openrouter(settings: &CoachSettings, history: &[String]) -> Result<RecapOutput> {
    let model = settings.openrouter_model()?;
    let body = json!({
        "model": model,
        "temperature": 0.2,
        "max_tokens": 800,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": RECAP_PROMPT },
            { "role": "user", "content": recap_user_prompt(history) }
        ]
    });
    let res = http()
        .post("https://openrouter.ai/api/v1/chat/completions")
        .timeout(Duration::from_secs(30))
        .bearer_auth(&settings.openrouter_key)
        .header("HTTP-Referer", "https://sonicsales.app")
        .header("X-Title", "Ghost Coach")
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    let v: Value = res.json().await?;
    if !status.is_success() {
        let message = v["error"]["message"]
            .as_str()
            .or_else(|| v["error"]["metadata"]["raw"].as_str())
            .unwrap_or("request failed");
        bail!("openrouter HTTP {status}: {message}");
    }
    parse_recap(
        v["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("{}"),
    )
}

async fn recap_gemini(settings: &CoachSettings, history: &[String]) -> Result<RecapOutput> {
    let prompt = format!("{RECAP_PROMPT}\n\n{}", recap_user_prompt(history));
    let body = json!({
        "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
        "generationConfig": {
            "temperature": 0.2,
            "maxOutputTokens": 500,
            "responseMimeType": "application/json"
        }
    });
    let mut last_err = String::from("no Gemini model tried");
    for model in settings.gemini_models() {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={}",
            settings.google_api_key
        );
        let res = http().post(&url).json(&body).send().await?;
        let status = res.status();
        let v: Value = res.json().await.unwrap_or(json!({}));
        if status.as_u16() == 404 {
            last_err = format!("{model} not found");
            continue;
        }
        if !status.is_success() {
            bail!(
                "gemini {model} HTTP {status}: {}",
                v["error"]["message"].as_str().unwrap_or("request failed")
            );
        }
        if let Ok(mut last) = LAST_GEMINI.lock() {
            *last = Some(model);
        }
        return parse_recap(
            v["candidates"][0]["content"]["parts"][0]["text"]
                .as_str()
                .unwrap_or("{}"),
        );
    }
    bail!("gemini models unavailable: {last_err}");
}

pub async fn suggest(
    settings: &CoachSettings,
    prospect: &str,
    history: &[String],
    brief: &str,
    summary: &str,
    open_questions: &[String],
    recent_coach: &[String],
) -> Result<CoachReply> {
    match settings.provider.as_str() {
        "openai" => {
            suggest_openai(
                settings,
                prospect,
                history,
                brief,
                summary,
                open_questions,
                recent_coach,
            )
            .await
        }
        "openrouter" => {
            suggest_openrouter(
                settings,
                prospect,
                history,
                brief,
                summary,
                open_questions,
                recent_coach,
            )
            .await
        }
        _ => {
            suggest_gemini(
                settings,
                prospect,
                history,
                brief,
                summary,
                open_questions,
                recent_coach,
            )
            .await
        }
    }
}

async fn suggest_openai(
    settings: &CoachSettings,
    prospect: &str,
    history: &[String],
    brief: &str,
    summary: &str,
    open_questions: &[String],
    recent_coach: &[String],
) -> Result<CoachReply> {
    let system = if is_sales_mode(brief, open_questions) {
        SALES_SYSTEM_PROMPT
    } else {
        GENERAL_SYSTEM_PROMPT
    };
    let body = json!({
        "model": settings.openai_model(),
        "temperature": 0.2,
        "max_tokens": 140,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user_prompt(prospect, history, brief, summary, open_questions, recent_coach) }
        ]
    });
    let res = http()
        .post("https://api.openai.com/v1/chat/completions")
        .bearer_auth(&settings.openai_key)
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    let v: Value = res.json().await?;
    if !status.is_success() {
        bail!(
            "openai HTTP {status}: {}",
            v["error"]["message"].as_str().unwrap_or("request failed")
        );
    }
    let content = v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("{}");
    parse_reply(content, prospect)
}

async fn suggest_openrouter(
    settings: &CoachSettings,
    prospect: &str,
    history: &[String],
    brief: &str,
    summary: &str,
    open_questions: &[String],
    recent_coach: &[String],
) -> Result<CoachReply> {
    let system = if is_sales_mode(brief, open_questions) {
        SALES_SYSTEM_PROMPT
    } else {
        GENERAL_SYSTEM_PROMPT
    };
    let model = settings.openrouter_model()?;
    let mut body = json!({
        "model": model,
        "temperature": 0.2,
        "max_tokens": 800,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": system },
            { "role": "user", "content": user_prompt(prospect, history, brief, summary, open_questions, recent_coach) }
        ]
    });
    if model.contains("gpt-6") {
        body["reasoning"] = json!({ "effort": "none" });
        body["max_completion_tokens"] = json!(800);
    }
    let res = http()
        .post("https://openrouter.ai/api/v1/chat/completions")
        .timeout(Duration::from_secs(20))
        .bearer_auth(&settings.openrouter_key)
        .header("HTTP-Referer", "https://sonicsales.app")
        .header("X-Title", "Ghost Coach")
        .json(&body)
        .send()
        .await?;
    let status = res.status();
    let v: Value = res.json().await?;
    if !status.is_success() {
        let message = v["error"]["message"]
            .as_str()
            .or_else(|| v["error"]["metadata"]["raw"].as_str())
            .unwrap_or("request failed");
        bail!("openrouter HTTP {status}: {message}");
    }
    let content = v["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("{}");
    parse_reply(content, prospect)
}

async fn suggest_gemini(
    settings: &CoachSettings,
    prospect: &str,
    history: &[String],
    brief: &str,
    summary: &str,
    open_questions: &[String],
    recent_coach: &[String],
) -> Result<CoachReply> {
    let system = if is_sales_mode(brief, open_questions) {
        SALES_SYSTEM_PROMPT
    } else {
        GENERAL_SYSTEM_PROMPT
    };
    let prompt = format!(
        "{system}\n\n{}",
        user_prompt(
            prospect,
            history,
            brief,
            summary,
            open_questions,
            recent_coach
        )
    );
    let body = json!({
        "contents": [{ "role": "user", "parts": [{ "text": prompt }] }],
        "generationConfig": {
            "temperature": 0.2,
            "maxOutputTokens": 120,
            "responseMimeType": "application/json"
        }
    });

    let mut last_err = String::from("no Gemini model tried");
    for model in settings.gemini_models() {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent?key={}",
            settings.google_api_key
        );
        let res = http().post(&url).json(&body).send().await?;
        let status = res.status();
        let v: Value = res.json().await.unwrap_or(json!({}));
        if status.as_u16() == 404 {
            last_err = format!("{model} not found");
            continue;
        }
        if !status.is_success() {
            bail!(
                "gemini {model} HTTP {status}: {}",
                v["error"]["message"].as_str().unwrap_or("request failed")
            );
        }
        let content = v["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or("{}");
        if let Ok(mut last) = LAST_GEMINI.lock() {
            *last = Some(model);
        }
        return parse_reply(content, prospect);
    }
    bail!("gemini models unavailable: {last_err}");
}

fn parse_reply(raw: &str, prospect: &str) -> Result<CoachReply> {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let parsed: Value = serde_json::from_str(trimmed)
        .map_err(|_| anyhow::anyhow!("The coach returned an invalid suggestion."))?;
    let question = parsed
        .get("question")
        .and_then(|x| x.as_str())
        .unwrap_or(prospect)
        .trim()
        .to_string();
    let mut answer = parsed
        .get("answer")
        .or_else(|| parsed.get("line"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    if answer.is_empty() {
        answer = "What is the biggest thing you want to improve right now?".into();
    }
    if answer.len() > 320 {
        answer = answer.chars().take(280).collect();
    }
    let allowed = [
        "Rapport",
        "Discovery",
        "Pain",
        "Impact",
        "Vision",
        "Offer",
        "Objection",
        "Close",
        "Next step",
        "Hold",
    ];
    let raw_tag = parsed
        .get("tag")
        .and_then(|x| x.as_str())
        .unwrap_or("Discovery")
        .trim();
    let tag = allowed
        .iter()
        .find(|item| item.eq_ignore_ascii_case(raw_tag))
        .copied()
        .unwrap_or("Discovery")
        .to_string();
    Ok(CoachReply {
        question: if question.is_empty() {
            prospect.to_string()
        } else {
            question
        },
        answer,
        why: parsed
            .get("why")
            .and_then(|x| x.as_str())
            .unwrap_or("Next line")
            .trim()
            .to_string(),
        tag,
    })
}

#[cfg(test)]
mod model_tests {
    use super::{is_sales_mode, objection_hint, user_prompt, CoachSettings};
    fn settings(model: &str) -> CoachSettings {
        CoachSettings {
            provider: "openrouter".into(),
            google_api_key: String::new(),
            google_model: String::new(),
            openai_key: String::new(),
            openai_model: String::new(),
            openrouter_key: "example-key".into(),
            openrouter_model: model.into(),
        }
    }
    #[test]
    fn manual_model_ids_are_not_replaced_by_presets() {
        assert_eq!(
            settings(" vendor/custom-model:free ")
                .openrouter_model()
                .unwrap(),
            "vendor/custom-model:free"
        );
        assert_eq!(
            settings("gpt-6-luna").openrouter_model().unwrap(),
            "gpt-6-luna"
        );
    }
    #[test]
    fn blank_model_requires_user_input() {
        assert!(settings(" ").openrouter_model().is_err());
    }

    #[test]
    fn sales_mode_survives_after_the_checklist_is_finished() {
        assert!(is_sales_mode("Call type: Plumbing sales call", &[]));
        assert!(!is_sales_mode("Call type: General meeting", &[]));
    }

    #[test]
    fn flags_common_plumber_objections_without_misfiring_on_busy_work() {
        assert!(objection_hint("I already have a website, but it does not bring calls")
            .unwrap()
            .contains("existing site"));
        assert!(objection_hint("That sounds too expensive for me")
            .unwrap()
            .contains("Price or budget"));
        assert!(objection_hint("I have too much work already")
            .unwrap()
            .contains("do not need more volume"));
    }

    #[test]
    fn prompt_includes_objection_read_and_prior_suggestions() {
        let prompt = user_prompt(
            "I already have a website and it does not bring calls",
            &[],
            "Call type: Plumbing sales call",
            "",
            &["What is bringing you most of your jobs right now?".into()],
            &["Would it be worth reviewing the site together?".into()],
        );
        assert!(prompt.contains("Ask whether it brings in calls"));
        assert!(prompt.contains("Would it be worth reviewing the site together?"));
        assert!(prompt.contains("DISCOVERY CHECKLIST STILL OPEN"));
    }
}
