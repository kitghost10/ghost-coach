//! Playbook question tracker: watches the live transcript and keeps
//! score of which playbook questions were asked and which were answered.
//!
//! Detection is mechanical (no AI calls): a question counts as ASKED when
//! a spoken line overlaps enough of its significant tokens, and as
//! ANSWERED when the prospect replies to it with a substantive line.

fn normalize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(stem)
        .collect()
}

/// Very light stemming so "customers" matches "customer", "decisions"
/// matches "decision". Only strips a trailing 's' on longer words.
fn stem(word: &str) -> String {
    if word.len() > 4 && word.ends_with('s') {
        word[..word.len() - 1].to_string()
    } else {
        word.to_string()
    }
}

const STOPWORDS: &[&str] = &[
    "what", "your", "you", "are", "the", "and", "for", "with", "this", "that",
    "have", "how", "who", "when", "where", "would", "could", "does", "about",
    "into", "over", "them", "they", "like", "just", "some", "much", "many",
    "from", "there", "then", "than", "been", "were", "will", "want",
];

fn significant_tokens(text: &str) -> Vec<String> {
    normalize(text)
        .into_iter()
        .filter(|w| w.len() >= 4 && !STOPWORDS.contains(&w.as_str()))
        .collect()
}

/// Fraction of the question's significant tokens present in the line.
fn ask_overlap(question: &str, line: &str) -> bool {
    let q_tokens = significant_tokens(question);
    if q_tokens.is_empty() {
        return false;
    }
    let line_tokens = normalize(line);
    let hits = q_tokens
        .iter()
        .filter(|q| line_tokens.contains(q))
        .count();
    // Short questions need most tokens; long ones need a strong minority.
    let needed = if q_tokens.len() <= 2 {
        q_tokens.len()
    } else {
        (q_tokens.len() + 1) / 2
    };
    hits >= needed
}

const ANSWER_WORDS: &[&str] = &[
    "yes", "yeah", "yep", "sure", "ok", "okay", "no", "not", "maybe",
    "probably", "definitely", "absolutely", "right", "exactly", "true",
];

fn looks_like_answer(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.ends_with('?') {
        return false; // they asked their own question instead of answering
    }
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.is_empty() {
        return false;
    }
    // The pending window opens right after you ask a question, so any
    // substantive reply counts; single words must be clear yes/no words.
    words.len() >= 2
        || words.iter().any(|w| {
            ANSWER_WORDS.contains(
                &w.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
                    .as_str(),
            )
        })
}

#[derive(Clone)]
struct TrackedQuestion {
    text: String,
    asked: bool,
    answered: bool,
}

#[derive(Default)]
pub struct QuestionTracker {
    questions: Vec<TrackedQuestion>,
    /// Index of the most recent question detected as asked, still awaiting
    /// a reply from the prospect.
    pending: Option<usize>,
}

impl QuestionTracker {
    pub fn new(questions: Vec<String>) -> Self {
        Self {
            questions: questions
                .into_iter()
                .filter(|q| !q.trim().is_empty())
                .map(|text| TrackedQuestion {
                    text,
                    asked: false,
                    answered: false,
                })
                .collect(),
            pending: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.questions.is_empty()
    }

    /// Feed one final transcript line, already labeled ("You: …" / "Prospect: …").
    pub fn observe(&mut self, labeled_line: &str) {
        if self.questions.is_empty() {
            return;
        }
        let line = labeled_line.trim();
        if line.is_empty() {
            return;
        }
        let from_you = line.to_lowercase().starts_with("you:")
            || line.to_lowercase().starts_with("caller:");
        let spoken = match line.split_once(':') {
            Some((_, rest)) => rest.trim(),
            None => line,
        };

        if from_you {
            for (i, q) in self.questions.iter_mut().enumerate() {
                if !q.asked && ask_overlap(&q.text, spoken) {
                    q.asked = true;
                    if !q.answered {
                        self.pending = Some(i);
                    }
                    break;
                }
            }
        } else if let Some(i) = self.pending {
            if looks_like_answer(spoken) {
                self.questions[i].answered = true;
                self.pending = None;
            }
        }
    }

    /// Playbook questions that still have no answer, in original order.
    pub fn remaining(&self) -> Vec<String> {
        self.questions
            .iter()
            .filter(|q| !q.answered)
            .map(|q| q.text.clone())
            .collect()
    }

    pub fn progress(&self) -> (usize, usize) {
        let answered = self.questions.iter().filter(|q| q.answered).count();
        (answered, self.questions.len())
    }
}

#[cfg(test)]
mod tests {
    use super::QuestionTracker;

    fn tracker() -> QuestionTracker {
        QuestionTracker::new(vec![
            "What is your main source of new customers today?".into(),
            "How much are you paying per lead right now?".into(),
            "Who else is involved in decisions like this?".into(),
        ])
    }

    #[test]
    fn detects_ask_and_answer_across_turns() {
        let mut t = tracker();
        t.observe("You: So what is your main source of new customers today?");
        assert_eq!(t.progress(), (0, 3));
        t.observe("Prospect: Mostly word of mouth and our Google reviews.");
        assert_eq!(t.progress(), (1, 3));
        let remaining = t.remaining();
        assert_eq!(remaining.len(), 2);
        assert!(!remaining[0].contains("source"));
    }

    #[test]
    fn detects_paraphrased_ask_with_stemming() {
        let mut t = tracker();
        t.observe("You: What is the main source of new customer for you right now?");
        t.observe("Prospect: Referrals, mostly.");
        assert_eq!(t.progress(), (1, 3));
    }

    #[test]
    fn prospect_question_does_not_count_as_answer() {
        let mut t = tracker();
        t.observe("You: How much are you paying per lead right now?");
        t.observe("Prospect: Why do you ask about that?");
        assert_eq!(t.progress(), (0, 3));
    }

    #[test]
    fn short_affirmative_replies_count_as_answers() {
        let mut t = QuestionTracker::new(vec![
            "If we could fix that, would you want to handle it this quarter?".into(),
        ]);
        t.observe("You: If we could fix that, would you want to handle it this quarter?");
        t.observe("Prospect: Yeah sure.");
        assert_eq!(t.progress(), (1, 1));
    }

    #[test]
    fn answered_question_is_not_remarked_by_later_overlap() {
        let mut t = tracker();
        t.observe("You: What is your main source of new customers today?");
        t.observe("Prospect: Word of mouth mostly.");
        // Same question repeated later must not reopen the pending state.
        t.observe("You: So the main source is new customers, got it.");
        assert!(t.pending.is_none());
        assert_eq!(t.progress(), (1, 3));
    }

    #[test]
    fn empty_tracker_is_inert() {
        let mut t = QuestionTracker::new(vec![]);
        t.observe("You: Anything at all?");
        t.observe("Prospect: Sure thing.");
        assert!(t.is_empty());
        assert!(t.remaining().is_empty());
    }
}
