//! The agent panel: cards an agent posts into the VBind window (notes, step
//! lists, questions with answer buttons) and controls it highlights.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use std::time::Duration;
use tokio::sync::watch;
use vbind_core::state::Guid;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Done,
    Active,
    #[default]
    Todo,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default)]
    pub status: StepStatus,
}

/// One card in the panel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Card {
    /// Markdown text (headings, lists, tables, code, links).
    Markdown {
        #[serde(default)]
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        markdown: String,
    },
    /// A checklist of setup steps with status.
    Steps {
        #[serde(default)]
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        steps: Vec<Step>,
    },
    /// A question the person answers with a button or a short reply.
    Question {
        #[serde(default)]
        id: String,
        question: String,
        #[serde(default)]
        options: Vec<String>,
        #[serde(default)]
        allow_text: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answer: Option<String>,
    },
}

impl Card {
    fn id_mut(&mut self) -> &mut String {
        match self {
            Card::Markdown { id, .. } | Card::Steps { id, .. } | Card::Question { id, .. } => id,
        }
    }
}

/// A control to draw attention to in the mapping and device views.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Highlight {
    /// Role in the active profile (`left`), or leave out and give `device`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<Guid>,
    /// Input control (`b1`, `hat1.up`, `y`), or an output key (`pad.a`) with no role/device.
    pub control: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PanelState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub cards: Vec<Card>,
    pub highlights: Vec<Highlight>,
    pub rev: u64,
}

/// What an agent sends to post to the panel. `markdown` and `steps` are
/// shorthands for a single card.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PanelPost {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub cards: Vec<Card>,
    #[serde(default)]
    pub markdown: Option<String>,
    #[serde(default)]
    pub steps: Option<Vec<Step>>,
    #[serde(default)]
    pub highlights: Option<Vec<Highlight>>,
    /// Add to the existing cards instead of replacing them. A card whose id
    /// matches an existing card replaces that card in place.
    #[serde(default)]
    pub append: bool,
}

pub struct Panel {
    state: Mutex<PanelState>,
    next_id: Mutex<u64>,
    changed: watch::Sender<u64>,
}

impl Default for Panel {
    fn default() -> Self {
        Panel { state: Mutex::new(PanelState::default()), next_id: Mutex::new(1), changed: watch::channel(0).0 }
    }
}

impl Panel {
    pub fn get(&self) -> PanelState {
        self.state.lock().unwrap().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    fn fresh_id(&self) -> String {
        let mut n = self.next_id.lock().unwrap();
        let id = format!("c{}", *n);
        *n += 1;
        id
    }

    fn bump(&self, st: &mut PanelState) {
        st.rev += 1;
        let _ = self.changed.send(st.rev);
    }

    pub fn post(&self, post: PanelPost) -> PanelState {
        let mut cards = post.cards;
        if let Some(md) = post.markdown {
            cards.insert(0, Card::Markdown { id: String::new(), title: None, markdown: md });
        }
        if let Some(steps) = post.steps {
            cards.push(Card::Steps { id: String::new(), title: None, steps });
        }
        for c in cards.iter_mut() {
            if c.id_mut().is_empty() {
                *c.id_mut() = self.fresh_id();
            }
        }
        let mut st = self.state.lock().unwrap();
        if post.append {
            for c in cards {
                let id = match &c {
                    Card::Markdown { id, .. } | Card::Steps { id, .. } | Card::Question { id, .. } => id.clone(),
                };
                match st.cards.iter_mut().find(|x| matches!(x, Card::Markdown { id: i, .. } | Card::Steps { id: i, .. } | Card::Question { id: i, .. } if *i == id)) {
                    Some(existing) => *existing = c,
                    None => st.cards.push(c),
                }
            }
        } else {
            st.cards = cards;
        }
        if post.title.is_some() || !post.append {
            st.title = post.title;
        }
        if let Some(h) = post.highlights {
            st.highlights = h;
        }
        self.bump(&mut st);
        st.clone()
    }

    pub fn clear(&self) {
        let mut st = self.state.lock().unwrap();
        st.cards.clear();
        st.highlights.clear();
        st.title = None;
        self.bump(&mut st);
    }

    /// Record the person's answer to a question card.
    pub fn answer(&self, id: &str, answer: String) -> bool {
        let mut st = self.state.lock().unwrap();
        let found = st.cards.iter_mut().any(|c| match c {
            Card::Question { id: cid, answer: a, .. } if cid == id => {
                *a = Some(answer.clone());
                true
            }
            _ => false,
        });
        if found {
            self.bump(&mut st);
        }
        found
    }

    /// Post a question and wait for the answer. Returns `None` on timeout.
    pub async fn ask(&self, question: String, options: Vec<String>, allow_text: bool, timeout: Duration) -> (String, Option<String>) {
        let id = self.fresh_id();
        let mut rx = self.subscribe();
        self.post(PanelPost {
            cards: vec![Card::Question { id: id.clone(), question, options, allow_text, answer: None }],
            append: true,
            ..Default::default()
        });
        let wait = async {
            loop {
                if let Some(a) = self.answer_of(&id) {
                    return Some(a);
                }
                if rx.changed().await.is_err() {
                    return None;
                }
            }
        };
        let answer = tokio::time::timeout(timeout, wait).await.ok().flatten();
        (id, answer)
    }

    fn answer_of(&self, id: &str) -> Option<String> {
        self.state.lock().unwrap().cards.iter().find_map(|c| match c {
            Card::Question { id: cid, answer, .. } if cid == id => answer.clone(),
            _ => None,
        })
    }
}
