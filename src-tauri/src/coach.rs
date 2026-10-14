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
        if model.is_empty() { bail!("Enter an OpenRouter model ID in Connections."); }
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

const SYSTEM_PROMPT: &str = "Silent sales copilot. JSON only: {\"question\":\"short\",\"answer\":\"one spoken sentence\",\"why\":\"max 6 words\",\"tag\":\"Discovery|Impact|Offer|Objection|Next step|Hold\"}. One concise next line. Never invent pricing or results.";

fn user_prompt(prospect: &str, history: &[String], brief: &str) -> String {
    let recent = history
        .iter()
        .rev()
        .take(4)
        .rev()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "BRIEF:{brief}\nMEMORY:\n{recent}\nTHEY SAID:\n{prospect}\n\nSALES GUIDANCE:\nPrioritize the next unanswered qualification question. Avoid repeating questions already present in MEMORY. If the prospect raises an objection, acknowledge it briefly before asking one clarifying question. Keep the suggested line natural to say aloud and under 20 words. Move toward a concrete next step when pain, value, authority, budget, and timing are sufficiently clear."
    )
}

pub async fn warmup(settings: &CoachSettings) {
    let _ = suggest(settings, "Hello", &[], "").await;
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
    parse_recap(v["choices"][0]["message"]["content"].as_str().unwrap_or("{}"))
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
    parse_recap(v["choices"][0]["message"]["content"].as_str().unwrap_or("{}"))
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
) -> Result<CoachReply> {
    match settings.provider.as_str() {
        "openai" => suggest_openai(settings, prospect, history, brief).await,
        "openrouter" => suggest_openrouter(settings, prospect, history, brief).await,
        _ => suggest_gemini(settings, prospect, history, brief).await,
    }
}

async fn suggest_openai(
    settings: &CoachSettings,
    prospect: &str,
    history: &[String], brief: &str,
) -> Result<CoachReply> {
    let body = json!({
        "model": settings.openai_model(),
        "temperature": 0.2,
        "max_tokens": 90,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": user_prompt(prospect, history, brief) }
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
    history: &[String], brief: &str,
) -> Result<CoachReply> {
    let model = settings.openrouter_model()?;
    let mut body = json!({
        "model": model,
        "temperature": 0.2,
        "max_tokens": 800,
        "response_format": { "type": "json_object" },
        "messages": [
            { "role": "system", "content": SYSTEM_PROMPT },
            { "role": "user", "content": user_prompt(prospect, history, brief) }
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
    history: &[String], brief: &str,
) -> Result<CoachReply> {
    let prompt = format!("{SYSTEM_PROMPT}\n\n{}", user_prompt(prospect, history, brief));
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
        answer = "What is the main thing you want more of from marketing right now?".into();
    }
    if answer.len() > 320 {
        answer = answer.chars().take(280).collect();
    }
    let allowed = ["Discovery", "Impact", "Offer", "Objection", "Next step", "Hold"];
    let raw_tag = parsed.get("tag").and_then(|x| x.as_str()).unwrap_or("Discovery").trim();
    let tag = allowed.iter().find(|item| item.eq_ignore_ascii_case(raw_tag)).copied().unwrap_or("Discovery").to_string();
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
    use super::CoachSettings;
    fn settings(model: &str) -> CoachSettings {
        CoachSettings { provider: "openrouter".into(), google_api_key: String::new(), google_model: String::new(), openai_key: String::new(), openai_model: String::new(), openrouter_key: "example-key".into(), openrouter_model: model.into() }
    }
    #[test]
    fn manual_model_ids_are_not_replaced_by_presets() {
        assert_eq!(settings(" vendor/custom-model:free ").openrouter_model().unwrap(), "vendor/custom-model:free");
        assert_eq!(settings("gpt-6-luna").openrouter_model().unwrap(), "gpt-6-luna");
    }
    #[test]
    fn blank_model_requires_user_input() {
        assert!(settings(" ").openrouter_model().is_err());
    }
}
