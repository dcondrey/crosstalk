//! Explicitly synthetic arithmetic fixtures for the topology simulator.
//!
//! These tasks do not exercise a model. They only give the simulated UCB1
//! scenarios a stable workload length and must never be reported as GSM8K
//! accuracy.

/// A synthetic arithmetic problem and its numeric answer.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MathProblem {
    pub question: String,
    /// Final numeric answer extracted from the `#### <number>` suffix.
    pub answer: f64,
}

/// Generate synthetic arithmetic problems for testing without the dataset file.
///
/// Produces deterministic problems of the form:
///   "A store sells {a} apples per box. {b} boxes were sold Monday.
///    {a} more were sold Tuesday. How many total?"
/// with answer = a × b + a.
pub fn synthetic_math_questions(n: usize) -> Vec<MathProblem> {
    (0..n)
        .map(|i| {
            let a = (i + 3) as f64;
            let b = (i + 7) as f64;
            MathProblem {
                question: format!(
                    "A store sells {a} apples per box and {b} boxes were sold on Monday. \
                     On Tuesday they sold {a} more apples. How many apples were sold in total?"
                ),
                answer: a * b + a,
            }
        })
        .collect()
}
