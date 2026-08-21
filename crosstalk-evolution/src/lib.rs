//! Native evolutionary discovery for Crosstalk.
//!
//! Generation and evaluation are supplied by callers. The crate owns the
//! deterministic evolutionary policy, lineage, retention, and checkpoints.

mod engine;
mod selection;
mod types;

pub use engine::{CandidateEvaluator, CandidateGenerator, EvolutionEngine, EvolutionError};
pub use selection::{blindmind_compatible_score, pareto_frontier, title_similarity};
pub use types::*;
