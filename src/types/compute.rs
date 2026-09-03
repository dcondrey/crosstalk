use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Spending tier that gates model selection and prompt verbosity.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
pub enum BudgetMode {
    /// More than 20 % of session budget remaining.
    Normal,
    /// Between 5 % and 20 % remaining; prefer cheaper models.
    CostReduction,
    /// Below 5 % remaining; cheapest path only.
    Emergency,
}

/// Token counts for a single API call.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
}

/// Per-turn cost record stored in the [`BudgetLedger`].
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CostEntry {
    pub turn_id: u32,
    pub model_id: String,
    pub usage: TokenUsage,
    /// Provider-reported or explicitly configured cost. Zero means unknown;
    /// Crosstalk never invents a nominal price for an unmetered response.
    pub cost_usd: f64,
    pub latency_ms: u64,
    pub timestamp: u64,
}

/// Tracks API spend across a session and derives the current [`BudgetMode`].
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct BudgetLedger {
    pub session_budget: f64,
    pub spent: f64,
    pub entries: Vec<CostEntry>,
    /// Hard ceilings. Zero means unlimited.
    #[serde(default)]
    pub max_model_calls: u64,
    #[serde(default)]
    pub max_input_tokens: u64,
    #[serde(default)]
    pub max_output_tokens: u64,
    /// Conservatively metered usage, including failed/retried calls.
    #[serde(default)]
    pub model_calls: u64,
    #[serde(default)]
    pub estimated_input_tokens: u64,
    #[serde(default)]
    pub estimated_output_tokens: u64,
}

impl BudgetLedger {
    #[must_use]
    pub fn remaining(&self) -> f64 {
        (self.session_budget - self.spent).max(0.0)
    }

    #[must_use]
    pub fn burn_rate(&self) -> f64 {
        if self.entries.is_empty() {
            return 0.0;
        }
        self.spent / self.entries.len() as f64
    }

    #[must_use]
    pub fn burn_rate_defined(&self) -> Option<f64> {
        if self.entries.is_empty() {
            return None;
        }
        Some(self.spent / self.entries.len() as f64)
    }

    #[must_use]
    pub fn mode(&self) -> BudgetMode {
        // session_budget == 0.0 means "no limit configured" — treat as Normal.
        if self.session_budget <= f64::EPSILON {
            return BudgetMode::Normal;
        }
        let pct = self.remaining() / self.session_budget;
        if pct < 0.05 {
            BudgetMode::Emergency
        } else if pct < 0.20 {
            BudgetMode::CostReduction
        } else {
            BudgetMode::Normal
        }
    }

    #[must_use]
    pub fn predict_final_cost(&self, total_expected_turns: u32) -> f64 {
        let burn = self.burn_rate();
        burn * f64::from(total_expected_turns)
    }

    pub fn is_at_risk(&self, total_expected_turns: u32) -> bool {
        self.predict_final_cost(total_expected_turns) > self.session_budget * 1.2
    }

    pub fn summary(&self) -> String {
        format!(
            "budget={:.4} spent={:.4} remaining={:.4} burn_rate={:.6} mode={:?} calls={}/{} input_tokens={}/{} output_tokens={}/{}",
            self.session_budget,
            self.spent,
            self.remaining(),
            self.burn_rate(),
            self.mode(),
            self.model_calls,
            display_limit(self.max_model_calls),
            self.estimated_input_tokens,
            display_limit(self.max_input_tokens),
            self.estimated_output_tokens,
            display_limit(self.max_output_tokens),
        )
    }

    /// Reserve a provider call before it starts. Failed attempts and retries
    /// remain charged because they consumed rate-limit and compute capacity.
    pub fn try_reserve_model_call(&mut self, estimated_input_tokens: u64) -> Result<(), String> {
        if self.max_model_calls > 0 && self.model_calls >= self.max_model_calls {
            return Err("session model-call limit exhausted".into());
        }
        if self.max_input_tokens > 0
            && self
                .estimated_input_tokens
                .saturating_add(estimated_input_tokens)
                > self.max_input_tokens
        {
            return Err("session input-token limit exhausted".into());
        }
        self.model_calls = self.model_calls.saturating_add(1);
        self.estimated_input_tokens = self
            .estimated_input_tokens
            .saturating_add(estimated_input_tokens);
        Ok(())
    }

    /// Reserve several model-call slots for a bounded subsystem such as native
    /// evolution. Returns the admitted count, which may be lower than
    /// requested when a hard session limit is configured.
    pub fn reserve_model_call_slots(&mut self, requested: u64) -> u64 {
        let admitted = if self.max_model_calls == 0 {
            requested
        } else {
            requested.min(self.max_model_calls.saturating_sub(self.model_calls))
        };
        self.model_calls = self.model_calls.saturating_add(admitted);
        admitted
    }

    pub fn release_unused_model_call_slots(&mut self, unused: u64) {
        self.model_calls = self.model_calls.saturating_sub(unused);
    }

    /// Charge streamed output incrementally so concurrent agents cannot each
    /// assume they own the same remaining allowance.
    pub fn try_consume_output_tokens(&mut self, estimated_tokens: u64) -> Result<(), String> {
        if self.max_output_tokens > 0
            && self
                .estimated_output_tokens
                .saturating_add(estimated_tokens)
                > self.max_output_tokens
        {
            return Err("session output-token limit exhausted".into());
        }
        self.estimated_output_tokens = self
            .estimated_output_tokens
            .saturating_add(estimated_tokens);
        Ok(())
    }
}

fn display_limit(value: u64) -> String {
    if value == 0 {
        "unlimited".into()
    } else {
        value.to_string()
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct ModelCapabilityMatrix {
    pub scores: BTreeMap<String, BTreeMap<String, f64>>,
}

impl ModelCapabilityMatrix {
    #[must_use]
    pub fn score(&self, model_id: &str, capability: &str) -> f64 {
        self.scores
            .get(model_id)
            .and_then(|caps| caps.get(capability))
            .copied()
            .unwrap_or(0.0)
    }

    pub fn register(&mut self, model_id: &str, capability: &str, score: f64) {
        self.scores
            .entry(model_id.to_string())
            .or_default()
            .insert(capability.to_string(), score.clamp(0.0, 1.0));
    }
}
