//! Domain-general deliberation protocols.
//!
//! This module deliberately contains no model-provider logic.  It turns a task
//! into an explicit epistemic contract that every provider can follow and that
//! the synthesizer can evaluate consistently.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReasoningDomain {
    Cryptanalysis,
    Decipherment,
    HistoricalInquiry,
    NaturalScience,
    Debate,
    Theoretical,
    Invention,
    Empirical,
    Decision,
    Software,
    General,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceStandard {
    FormalProof,
    ReproducibleExperiment,
    SourceTriangulation,
    ArgumentMap,
    EngineeringValidation,
    ExplicitUncertainty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliberationProtocol {
    pub domain: ReasoningDomain,
    pub roles: Vec<String>,
    pub phases: Vec<String>,
    pub evidence: Vec<EvidenceStandard>,
    pub completion_contract: Vec<String>,
}

impl DeliberationProtocol {
    #[must_use]
    pub fn for_task(task: &str) -> Self {
        match classify_task(task) {
            ReasoningDomain::Cryptanalysis => Self::cryptanalysis(),
            ReasoningDomain::Decipherment => Self::decipherment(),
            ReasoningDomain::HistoricalInquiry => Self::historical_inquiry(),
            ReasoningDomain::NaturalScience => Self::natural_science(),
            ReasoningDomain::Debate => Self::debate(),
            ReasoningDomain::Theoretical => Self::theoretical(),
            ReasoningDomain::Invention => Self::invention(),
            ReasoningDomain::Empirical => Self::empirical(),
            ReasoningDomain::Decision => Self::decision(),
            ReasoningDomain::Software => Self::software(),
            ReasoningDomain::General => Self::general(),
        }
    }

    fn cryptanalysis() -> Self {
        Self {
            domain: ReasoningDomain::Cryptanalysis,
            roles: roles(&[
                "cryptanalyst",
                "statistician",
                "implementation auditor",
                "independent verifier",
            ]),
            phases: phases(&[
                "Establish lawful scope, exact ciphertext, metadata, and success criteria",
                "Identify encoding, cipher family, entropy, periodicity, and structural leakage",
                "Generate competing hypotheses without fitting a preferred plaintext",
                "Test hypotheses with reproducible scripts, known-answer checks, and null models",
                "Independently reproduce the best candidate and quantify ambiguity",
            ]),
            evidence: vec![
                EvidenceStandard::ReproducibleExperiment,
                EvidenceStandard::FormalProof,
            ],
            completion_contract: phases(&[
                "Inputs and transformations are preserved byte-for-byte",
                "A candidate beats explicit random and alternative-cipher baselines",
                "The result is reproducible and uncertainty is reported",
            ]),
        }
    }

    fn decipherment() -> Self {
        Self {
            domain: ReasoningDomain::Decipherment,
            roles: roles(&[
                "historical linguist",
                "epigrapher",
                "corpus statistician",
                "archaeological contextualist",
                "skeptic",
            ]),
            phases: phases(&[
                "Preserve sign readings, provenance, damage, direction, and segmentation uncertainty",
                "Build sign inventories, distributions, positional patterns, and repeated formulae",
                "Compare typological and bilingual-anchor hypotheses without assuming ancestry",
                "Predict unseen inscriptions or withheld segments from each hypothesis",
                "Seek independent archaeological and linguistic corroboration",
            ]),
            evidence: vec![
                EvidenceStandard::ReproducibleExperiment,
                EvidenceStandard::SourceTriangulation,
            ],
            completion_contract: phases(&[
                "Restorations and uncertain signs remain explicitly marked",
                "The reading generalizes to held-out material",
                "Competing decipherments and degrees of underdetermination are retained",
            ]),
        }
    }

    fn historical_inquiry() -> Self {
        Self {
            domain: ReasoningDomain::HistoricalInquiry,
            roles: roles(&[
                "archivist",
                "historian",
                "forensic timeline analyst",
                "source critic",
                "alternative-hypothesis advocate",
            ]),
            phases: phases(&[
                "Separate primary evidence, later testimony, interpretation, and folklore",
                "Construct a provenance-aware timeline and identify missing records",
                "Generate mutually exclusive hypotheses and their expected evidence",
                "Test source independence, incentives, anachronisms, and chain of custody",
                "Rank explanations by explanatory reach without converting absence into proof",
            ]),
            evidence: vec![
                EvidenceStandard::SourceTriangulation,
                EvidenceStandard::ArgumentMap,
            ],
            completion_contract: phases(&[
                "Every material claim is traceable to a source class",
                "Facts, inference, and speculation are visually separable",
                "The conclusion lists decisive missing evidence and viable alternatives",
            ]),
        }
    }

    fn natural_science() -> Self {
        Self {
            domain: ReasoningDomain::NaturalScience,
            roles: roles(&[
                "theorist",
                "experimentalist",
                "measurement specialist",
                "statistician",
                "replication critic",
            ]),
            phases: phases(&[
                "State the phenomenon, units, boundary conditions, and established constraints",
                "Derive competing mechanistic models from conserved quantities and known interactions",
                "Calculate limiting cases, dimensional checks, and quantitative predictions",
                "Design controls and a discriminating experiment or simulation",
                "Assess measurement error, confounding, safety, and replication",
            ]),
            evidence: vec![
                EvidenceStandard::ReproducibleExperiment,
                EvidenceStandard::FormalProof,
                EvidenceStandard::SourceTriangulation,
            ],
            completion_contract: phases(&[
                "Predictions have units, ranges, and stated assumptions",
                "At least one observation can distinguish the leading models",
                "Simulation, mathematical derivation, and empirical confirmation are not conflated",
            ]),
        }
    }

    #[must_use]
    pub fn prompt_contract(&self) -> String {
        let roles = self.roles.join(", ");
        let phases = self
            .phases
            .iter()
            .enumerate()
            .map(|(i, p)| format!("{}. {p}", i + 1))
            .collect::<Vec<_>>()
            .join("\n");
        let completion = self
            .completion_contract
            .iter()
            .map(|c| format!("- {c}"))
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "[EPISTEMIC PROTOCOL: {:?}]\nRoles: {roles}\nPhases:\n{phases}\nCompletion requires:\n{completion}\nPrefix material claims with [FACT], [ASSUMPTION], [INFERENCE], [CONJECTURE], or [PROPOSAL] so they enter the claim ledger. Separate these categories rigorously. Calibrate confidence and state what evidence would change the conclusion.",
            self.domain
        )
    }

    fn debate() -> Self {
        Self {
            domain: ReasoningDomain::Debate,
            roles: roles(&["proponent", "opponent", "steelman", "judge"]),
            phases: phases(&[
                "Define the resolution, terms, burdens of proof, and shared premises",
                "Construct the strongest independent cases on both sides",
                "Cross-examine pivotal premises and surface cruxes",
                "Steelman the opposing case before rebutting it",
                "Judge claims individually and report residual disagreement",
            ]),
            evidence: vec![
                EvidenceStandard::ArgumentMap,
                EvidenceStandard::SourceTriangulation,
            ],
            completion_contract: phases(&[
                "Every decisive premise has support or is labelled an assumption",
                "The result names the cruxes rather than using majority vote",
                "The final judgment reports confidence and strongest counterargument",
            ]),
        }
    }

    fn theoretical() -> Self {
        Self {
            domain: ReasoningDomain::Theoretical,
            roles: roles(&[
                "formalizer",
                "constructor",
                "counterexample hunter",
                "proof auditor",
            ]),
            phases: phases(&[
                "Normalize definitions, quantifiers, assumptions, and target",
                "Check consistency and search small or boundary counterexamples",
                "Develop independent proof strategies or derive an impossibility",
                "Translate the winning argument into a proof-assistant-ready statement",
                "Kernel-check the proof; distinguish theorem, conjecture, and evidence",
            ]),
            evidence: vec![
                EvidenceStandard::FormalProof,
                EvidenceStandard::ExplicitUncertainty,
            ],
            completion_contract: phases(&[
                "No hidden quantifiers or undefined terms remain",
                "Edge cases and counterexamples were actively searched",
                "A theorem is called proven only after a trusted checker accepts it",
            ]),
        }
    }

    fn invention() -> Self {
        Self {
            domain: ReasoningDomain::Invention,
            roles: roles(&[
                "scientist",
                "systems inventor",
                "prior-art skeptic",
                "experimentalist",
                "safety critic",
            ]),
            phases: phases(&[
                "Extract constraints, objective metrics, and forbidden failure modes",
                "Decompose the system by function and underlying physical principle",
                "Generate multiple orthogonal mechanisms, including reversals and analogies",
                "Combine compatible mechanisms and identify the novel technical delta",
                "Falsify with first-principles checks, prior-art risk, and red-team analysis",
                "Specify the cheapest decisive prototype and staged validation plan",
            ]),
            evidence: vec![
                EvidenceStandard::ReproducibleExperiment,
                EvidenceStandard::EngineeringValidation,
            ],
            completion_contract: phases(&[
                "The proposal includes mechanism, not just desired behavior",
                "Novelty, feasibility, safety, cost, and scale are scored separately",
                "Kill criteria and a falsifiable prototype are specified",
            ]),
        }
    }

    fn empirical() -> Self {
        Self {
            domain: ReasoningDomain::Empirical,
            roles: roles(&["researcher", "replicator", "statistician", "skeptic"]),
            phases: phases(&[
                "Operationalize the question",
                "Gather independent evidence",
                "Assess bias and alternatives",
                "Propose a discriminating test",
            ]),
            evidence: vec![
                EvidenceStandard::ReproducibleExperiment,
                EvidenceStandard::SourceTriangulation,
            ],
            completion_contract: phases(&[
                "Claims are traceable",
                "Correlation and causation are distinguished",
                "Uncertainty is quantified where possible",
            ]),
        }
    }

    fn decision() -> Self {
        Self {
            domain: ReasoningDomain::Decision,
            roles: roles(&["decision analyst", "domain expert", "risk critic"]),
            phases: phases(&[
                "Define objectives and constraints",
                "Generate options",
                "Model tradeoffs and uncertainty",
                "Test sensitivity and reversibility",
            ]),
            evidence: vec![
                EvidenceStandard::ArgumentMap,
                EvidenceStandard::ExplicitUncertainty,
            ],
            completion_contract: phases(&[
                "Dominated options are identified",
                "Recommendation changes are tied to explicit assumptions",
                "Reversible next action is clear",
            ]),
        }
    }

    fn software() -> Self {
        Self {
            domain: ReasoningDomain::Software,
            roles: roles(&["implementer", "reviewer", "tester"]),
            phases: phases(&[
                "Reproduce and specify",
                "Design",
                "Implement",
                "Test and inspect regressions",
            ]),
            evidence: vec![EvidenceStandard::EngineeringValidation],
            completion_contract: phases(&[
                "Acceptance criteria pass",
                "Failure paths are tested",
                "The change is scoped and explainable",
            ]),
        }
    }

    fn general() -> Self {
        Self {
            domain: ReasoningDomain::General,
            roles: roles(&["analyst", "skeptic", "synthesizer"]),
            phases: phases(&[
                "Clarify the objective",
                "Develop independent answers",
                "Challenge assumptions",
                "Synthesize with uncertainty",
            ]),
            evidence: vec![EvidenceStandard::ExplicitUncertainty],
            completion_contract: phases(&[
                "The answer addresses the actual objective",
                "Key assumptions and limitations are visible",
            ]),
        }
    }
}

fn roles(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_string()).collect()
}
fn phases(values: &[&str]) -> Vec<String> {
    roles(values)
}

#[must_use]
pub fn classify_task(task: &str) -> ReasoningDomain {
    let t = task.to_ascii_lowercase();
    let has = |words: &[&str]| words.iter().any(|word| t.contains(word));
    if has(&[
        "cryptographic puzzle",
        "cryptanalysis",
        "ciphertext",
        "decode this cipher",
        "break this cipher",
    ]) {
        ReasoningDomain::Cryptanalysis
    } else if has(&[
        "ancient language",
        "undeciphered",
        "inscription",
        "decipherment",
        "unknown script",
        "epigraphy",
    ]) {
        ReasoningDomain::Decipherment
    } else if has(&[
        "historical mystery",
        "unsolved mystery",
        "historical evidence",
        "archive",
        "what happened to",
    ]) {
        ReasoningDomain::HistoricalInquiry
    } else if has(&[
        "physics",
        "chemistry",
        "biology",
        "molecule",
        "protein",
        "quantum",
        "thermodynamic",
        "particle",
    ]) {
        ReasoningDomain::NaturalScience
    } else if has(&[
        "debate",
        "argue",
        "resolution",
        "case for",
        "case against",
        "steelman",
    ]) {
        ReasoningDomain::Debate
    } else if has(&[
        "theorem",
        "prove",
        "proof",
        "lemma",
        "conjecture",
        "mathematical",
        "formalize",
        "formalise",
    ]) {
        ReasoningDomain::Theoretical
    } else if has(&[
        "invent",
        "novel technology",
        "new mechanism",
        "patent",
        "prototype",
        "breakthrough",
        "brainstorm",
    ]) {
        ReasoningDomain::Invention
    } else if has(&[
        "experiment",
        "empirical",
        "study",
        "evidence",
        "causal",
        "hypothesis",
    ]) {
        ReasoningDomain::Empirical
    } else if has(&[
        "choose",
        "decision",
        "trade-off",
        "tradeoff",
        "compare options",
        "recommend",
    ]) {
        ReasoningDomain::Decision
    } else if has(&[
        "code",
        "debug",
        "implement",
        "refactor",
        "compiler",
        "api",
        "software",
        "test failure",
    ]) {
        ReasoningDomain::Software
    } else {
        ReasoningDomain::General
    }
}
