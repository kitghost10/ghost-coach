use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct Meters {
    pub you: f32,
    pub them: f32,
}

#[derive(Serialize, Clone)]
pub struct TranscriptEvt {
    pub kind: String,
    pub text: String,
    pub speaker: String,
}

#[derive(Serialize, Clone)]
pub struct CoachEvt {
    pub question: String,
    pub answer: String,
    pub why: String,
    pub tag: String,
}

#[derive(Serialize, Clone)]
pub struct StatusEvt {
    pub text: String,
    pub live: bool,
}

#[derive(Serialize, Clone)]
pub struct SessionUi {
    pub auto_answer: bool,
    pub coach_ready: bool,
}
