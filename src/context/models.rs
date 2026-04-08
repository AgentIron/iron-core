use serde::{Deserialize, Serialize};

pub const HANDOFF_DEFAULT_TARGET_TOKENS: usize = 15_000;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CompactedContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_step: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_constraints: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub established_facts: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environmental_assumptions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decisions: Option<Vec<Decision>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unresolved_questions: Option<Vec<UnresolvedQuestion>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recent_results: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub portability_notes: Option<Vec<PortabilityNote>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl CompactedContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_objective(mut self, objective: impl Into<String>) -> Self {
        self.objective = Some(objective.into());
        self
    }

    pub fn with_next_step(mut self, step: impl Into<String>) -> Self {
        self.next_step = Some(step.into());
        self
    }

    pub fn add_user_constraint(mut self, constraint: impl Into<String>) -> Self {
        self.user_constraints
            .get_or_insert_with(Vec::new)
            .push(constraint.into());
        self
    }

    pub fn add_fact(mut self, fact: impl Into<String>) -> Self {
        self.established_facts
            .get_or_insert_with(Vec::new)
            .push(fact.into());
        self
    }

    pub fn add_decision(mut self, decision: Decision) -> Self {
        self.decisions.get_or_insert_with(Vec::new).push(decision);
        self
    }

    pub fn add_unresolved_question(mut self, question: UnresolvedQuestion) -> Self {
        self.unresolved_questions
            .get_or_insert_with(Vec::new)
            .push(question);
        self
    }

    pub fn add_recent_result(mut self, result: impl Into<String>) -> Self {
        self.recent_results
            .get_or_insert_with(Vec::new)
            .push(result.into());
        self
    }

    pub fn add_portability_note(mut self, note: PortabilityNote) -> Self {
        self.portability_notes
            .get_or_insert_with(Vec::new)
            .push(note);
        self
    }

    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        self.notes = Some(notes.into());
        self
    }

    pub fn is_empty(&self) -> bool {
        self.objective.is_none()
            && self.next_step.is_none()
            && self.user_constraints.as_ref().is_none_or(|v| v.is_empty())
            && self.established_facts.as_ref().is_none_or(|v| v.is_empty())
            && self
                .environmental_assumptions
                .as_ref()
                .is_none_or(|v| v.is_empty())
            && self.decisions.as_ref().is_none_or(|v| v.is_empty())
            && self
                .unresolved_questions
                .as_ref()
                .is_none_or(|v| v.is_empty())
            && self.recent_results.as_ref().is_none_or(|v| v.is_empty())
            && self.portability_notes.as_ref().is_none_or(|v| v.is_empty())
            && self.notes.is_none()
    }

    pub fn render_to_text(&self) -> String {
        let mut parts = Vec::new();
        if let Some(ref obj) = self.objective {
            parts.push(format!("Objective: {}", obj));
        }
        if let Some(ref step) = self.next_step {
            parts.push(format!("Next step: {}", step));
        }
        if let Some(ref facts) = self.established_facts {
            if !facts.is_empty() {
                parts.push(format!("Established facts: {}", facts.join("; ")));
            }
        }
        if let Some(ref constraints) = self.user_constraints {
            if !constraints.is_empty() {
                parts.push(format!("User constraints: {}", constraints.join("; ")));
            }
        }
        if let Some(ref assumptions) = self.environmental_assumptions {
            if !assumptions.is_empty() {
                parts.push(format!(
                    "Environmental assumptions: {}",
                    assumptions.join("; ")
                ));
            }
        }
        if let Some(ref decisions) = self.decisions {
            let rendered: Vec<String> = decisions
                .iter()
                .map(|d| {
                    if let Some(ref r) = d.rationale {
                        format!("{} (because: {})", d.decision, r)
                    } else {
                        d.decision.clone()
                    }
                })
                .collect();
            if !rendered.is_empty() {
                parts.push(format!("Decisions: {}", rendered.join("; ")));
            }
        }
        if let Some(ref questions) = self.unresolved_questions {
            let rendered: Vec<String> = questions
                .iter()
                .map(|q| {
                    if q.blocking {
                        format!("{} [BLOCKING]", q.question)
                    } else {
                        q.question.clone()
                    }
                })
                .collect();
            if !rendered.is_empty() {
                parts.push(format!("Unresolved questions: {}", rendered.join("; ")));
            }
        }
        if let Some(ref results) = self.recent_results {
            if !results.is_empty() {
                parts.push(format!("Recent results: {}", results.join("; ")));
            }
        }
        if let Some(ref notes) = self.portability_notes {
            let rendered: Vec<String> = notes
                .iter()
                .map(|n| {
                    if n.non_portable {
                        format!(
                            "{} [non-portable: {}]",
                            n.note,
                            n.reason.as_deref().unwrap_or("unknown")
                        )
                    } else {
                        n.note.clone()
                    }
                })
                .collect();
            if !rendered.is_empty() {
                parts.push(format!("Portability notes: {}", rendered.join("; ")));
            }
        }
        if let Some(ref notes) = self.notes {
            parts.push(format!("Notes: {}", notes));
        }
        parts.join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

impl Decision {
    pub fn new(decision: impl Into<String>) -> Self {
        Self {
            decision: decision.into(),
            rationale: None,
        }
    }

    pub fn with_rationale(mut self, rationale: impl Into<String>) -> Self {
        self.rationale = Some(rationale.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UnresolvedQuestion {
    pub question: String,
    pub blocking: bool,
}

impl UnresolvedQuestion {
    pub fn new(question: impl Into<String>) -> Self {
        Self {
            question: question.into(),
            blocking: false,
        }
    }

    pub fn blocking(mut self) -> Self {
        self.blocking = true;
        self
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PortabilityNote {
    pub note: String,
    pub non_portable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl PortabilityNote {
    pub fn portable(note: impl Into<String>) -> Self {
        Self {
            note: note.into(),
            non_portable: false,
            reason: None,
        }
    }

    pub fn non_portable(note: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            note: note.into(),
            non_portable: true,
            reason: Some(reason.into()),
        }
    }
}
