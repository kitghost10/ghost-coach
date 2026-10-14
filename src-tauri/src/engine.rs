use crate::coach::CoachSettings;
use crate::config::OverlayPrefs;
use crate::playbook::QuestionTracker;
use crate::record;
use std::sync::atomic::{AtomicBool, AtomicU64};
use std::sync::{Arc, Mutex};

pub struct Engine {
    pub stop: Arc<AtomicBool>,
    pub preview_stop: Arc<AtomicBool>,
    pub session_gen: Arc<AtomicU64>,
    pub you_talking: Arc<AtomicBool>,
    pub last_prospect: Mutex<String>,
    pub history: Mutex<Vec<String>>,
    pub brief: Mutex<String>,
    pub coach: Mutex<Option<CoachSettings>>,
    pub auto_answer: Arc<AtomicBool>,
    pub transcript: Arc<Mutex<Vec<record::Entry>>>,
    pub sink: Arc<Mutex<Option<Arc<record::CallSink>>>>,
    pub overlay: Mutex<OverlayPrefs>,
    pub rolling_summary: Mutex<String>,
    pub questions: Mutex<QuestionTracker>,
}

impl Default for Engine {
    fn default() -> Self {
        Self {
            stop: Arc::new(AtomicBool::new(true)),
            preview_stop: Arc::new(AtomicBool::new(false)),
            session_gen: Arc::new(AtomicU64::new(0)),
            you_talking: Arc::new(AtomicBool::new(false)),
            last_prospect: Mutex::new(String::new()),
            history: Mutex::new(Vec::new()),
            brief: Mutex::new(String::new()),
            coach: Mutex::new(None),
            auto_answer: Arc::new(AtomicBool::new(true)),
            transcript: Arc::new(Mutex::new(Vec::new())),
            sink: Arc::new(Mutex::new(None)),
            overlay: Mutex::new(OverlayPrefs::default()),
            rolling_summary: Mutex::new(String::new()),
            questions: Mutex::new(QuestionTracker::default()),
        }
    }
}
