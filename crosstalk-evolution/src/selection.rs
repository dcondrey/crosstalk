use crate::{Concept, Fitness};
use std::collections::BTreeSet;

/// Reproduces BlindMind's current critic score for migration comparisons.
#[must_use]
pub fn blindmind_compatible_score(f: &Fitness) -> f64 {
    let novelty_bonus = (10.0 - f.prior_art_overlap).max(0.0) * 0.25;
    let raw = (f.feasibility * 1.5 + f.novelty + f.utility + f.semantic_jump * 0.5) / 5.0;
    raw + novelty_bonus - f.fatal_flaws.len().min(3) as f64 * 2.5
}

#[must_use]
pub fn title_similarity(a: &str, b: &str) -> f64 {
    let words = |value: &str| {
        value
            .to_ascii_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect::<BTreeSet<_>>()
    };
    let a = words(a);
    let b = words(b);
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    a.intersection(&b).count() as f64 / a.union(&b).count().max(1) as f64
}

fn dominates(a: &Concept, b: &Concept) -> bool {
    let av = [
        a.fitness.novelty,
        a.fitness.feasibility,
        a.fitness.utility,
        a.fitness.evidence,
        a.fitness.safety,
        10.0 - a.fitness.prior_art_overlap,
    ];
    let bv = [
        b.fitness.novelty,
        b.fitness.feasibility,
        b.fitness.utility,
        b.fitness.evidence,
        b.fitness.safety,
        10.0 - b.fitness.prior_art_overlap,
    ];
    av.iter().zip(&bv).all(|(x, y)| x >= y) && av.iter().zip(&bv).any(|(x, y)| x > y)
}

#[must_use]
pub fn pareto_frontier<'a>(concepts: impl IntoIterator<Item = &'a Concept>) -> Vec<&'a Concept> {
    let values: Vec<_> = concepts.into_iter().collect();
    values
        .iter()
        .copied()
        .filter(|candidate| {
            !values
                .iter()
                .any(|other| other.id != candidate.id && dominates(other, candidate))
        })
        .collect()
}
