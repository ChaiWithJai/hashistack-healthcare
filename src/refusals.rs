//! Refusal surface (#12, GOAL.md bar 7): the describe endpoint recognizes
//! the four out-of-scope shapes from RFC 0001 and refuses to scaffold them
//! — with a **written reason**, because "a refusal with a reason is a trust
//! feature" (RFC 0001, profile table). The four classes, by prior analysis:
//!
//! - **15 — triage liability:** symptom-routing that tells a *patient*
//!   whether to seek emergency care.
//! - **21 — FDA device:** software that reads clinical signals and renders
//!   a diagnosis is a regulated medical device (SaMD).
//! - **10 — ONC interoperability:** a certified interoperability
//!   hub/platform is a certification program, not a practice tool.
//! - **9 — enterprise outcomes platform:** cross-hospital benchmarking BI,
//!   not a clinician's tool.
//!
//! HONEST LABELING — the Phase 0 mechanism is keyword/pattern rules over
//! the normalized prompt (plus the pack the doctor grabbed, used for the
//! written reason and available to future shape checks). That is the same
//! deterministic-floor philosophy as the rules agent tier: cheap, testable
//! against the whole eval corpus (unit tests below run every committed
//! scenario prompt through the screen, both directions), and honest about
//! what it is. A model-based screen slots in *behind this same seam* later
//! — [`screen`]'s signature (prompt + pack in, `Option<Refusal>` out) is
//! exactly what a classifier tier needs, and the ladder's
//! verification-not-prediction rule applies to it too. Tracked in #12.
//!
//! The screen runs at describe time only: iterate instructions mutate an
//! admitted app under the gate engine's regression rules, which is a
//! different (already enforced) contract.

use crate::packs::PackManifest;

/// One refused describe: which RFC class fired and the written reason the
/// doctor sees (HTTP 422 body) and the audit stream records (`app.refused`).
#[derive(Debug)]
pub struct Refusal {
    pub rfc_use_case: u8,
    pub class: &'static str,
    pub reason: String,
}

/// One Phase 0 rule: every group must match at least one of its needles
/// (CNF over the lowercased prompt). Groups keep single hot words like
/// "triage" from firing alone — tuned against the eval corpus, where
/// legitimate prompts say "flag high ones", "escalation to the practice
/// inbox", and "red flags on top" without ever routing a patient.
struct Rule {
    rfc_use_case: u8,
    class: &'static str,
    all: &'static [&'static [&'static str]],
    explanation: &'static str,
}

const RULES: &[Rule] = &[
    // 15 — triage / emergency symptom-routing. Either the prompt names
    // triage in a patient-facing shape, or it routes patients to emergency
    // care in so many words.
    Rule {
        rfc_use_case: 15,
        class: "triage liability",
        all: &[
            &["triage"],
            &["patient", "symptom", "bot", "chest pain", "emergency"],
        ],
        explanation: "Routing patients on their symptoms is clinical triage — a liability \
                      the platform will not scaffold. Tracking observations and escalating \
                      them to the practice stays in scope; telling a patient whether to \
                      seek emergency care does not.",
    },
    Rule {
        rfc_use_case: 15,
        class: "triage liability",
        all: &[&[
            "go to the er",
            "go to the emergency",
            "call 911",
            "whether to seek emergency",
            "symptom checker",
        ]],
        explanation: "Routing patients on their symptoms is clinical triage — a liability \
                      the platform will not scaffold. Tracking observations and escalating \
                      them to the practice stays in scope; telling a patient whether to \
                      seek emergency care does not.",
    },
    // 21 — FDA device / diagnosis software: software rendering a diagnosis,
    // or reading a clinical signal source and interpreting it itself.
    Rule {
        rfc_use_case: 21,
        class: "FDA device (SaMD)",
        all: &[
            &["diagnos"],
            &[
                "automat",
                "reads",
                "read ",
                "detect",
                "interpret",
                "algorithm",
                "ai ",
            ],
        ],
        explanation: "Software that reads clinical signals and renders a diagnosis is a \
                      regulated medical device (FDA SaMD) — out of platform scope. \
                      Recording observations and flagging them to a clinician who decides \
                      stays in scope.",
    },
    Rule {
        rfc_use_case: 21,
        class: "FDA device (SaMD)",
        all: &[
            &["ecg", "ekg", "x-ray", "imaging", "radiograph"],
            &["diagnos", "detect", "interpret", "reads", "read "],
        ],
        explanation: "Software that reads clinical signals and renders a diagnosis is a \
                      regulated medical device (FDA SaMD) — out of platform scope. \
                      Recording observations and flagging them to a clinician who decides \
                      stays in scope.",
    },
    // 10 — ONC interoperability platform: the certification program shape,
    // or an every-hospital EHR sync hub.
    Rule {
        rfc_use_case: 10,
        class: "ONC interoperability",
        all: &[
            &["interoperability"],
            &["hub", "platform", "certif", "sync"],
        ],
        explanation: "An ONC-certified interoperability hub is a certification program, \
                      not a practice tool — out of platform scope.",
    },
    Rule {
        rfc_use_case: 10,
        class: "ONC interoperability",
        all: &[&[
            "onc-certified",
            "onc certified",
            "health information exchange",
        ]],
        explanation: "An ONC-certified interoperability hub is a certification program, \
                      not a practice tool — out of platform scope.",
    },
    // 9 — enterprise outcomes platform: cross-hospital benchmarking BI.
    Rule {
        rfc_use_case: 9,
        class: "enterprise outcomes platform",
        all: &[
            &[
                "benchmark",
                "risk-adjusted",
                "outcomes analytics",
                "outcomes platform",
            ],
            &["hospitals", "health system", "enterprise", "across"],
        ],
        explanation: "An enterprise outcomes-analytics platform benchmarking across \
                      hospital systems is enterprise BI, not a clinician's tool — out of \
                      platform scope.",
    },
];

/// The one verb: screen a describe request before anything is scaffolded.
/// `Some(refusal)` means the API answers 422 with the written reason and
/// records `app.refused`; `None` admits the prompt to the normal path.
pub fn screen(prompt: &str, pack: &PackManifest) -> Option<Refusal> {
    let normalized = prompt.to_lowercase();
    for rule in RULES {
        let fired = rule
            .all
            .iter()
            .all(|group| group.iter().any(|needle| normalized.contains(needle)));
        if fired {
            return Some(Refusal {
                rfc_use_case: rule.rfc_use_case,
                class: rule.class,
                reason: format!(
                    "refused: this request matches RFC 0001 use case {} ({}) — out of \
                     platform scope by prior analysis. {} A refusal with a reason is a \
                     trust feature (RFC 0001); the {} pack remains available for tools \
                     that observe, track, and escalate to the practice.",
                    rule.rfc_use_case, rule.class, rule.explanation, pack.id
                ),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::screen;
    use crate::packs::builtin_packs;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;
    use std::sync::OnceLock;

    /// Every committed eval scenario, keyed by category — the screen's
    /// tuning corpus. Reading the real files keeps these tests honest: a
    /// new scenario is screened the moment it is committed.
    fn corpus() -> BTreeMap<String, Vec<(String, String, Option<u64>)>> {
        static CORPUS: OnceLock<BTreeMap<String, Vec<(String, String, Option<u64>)>>> =
            OnceLock::new();
        CORPUS
            .get_or_init(|| {
                let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("evals/scenarios");
                let mut by_category = BTreeMap::new();
                let mut entries: Vec<_> = fs::read_dir(&dir)
                    .expect("evals/scenarios exists")
                    .map(|e| e.expect("readable dir entry").path())
                    .filter(|p| p.extension().is_some_and(|e| e == "json"))
                    .collect();
                entries.sort();
                assert!(!entries.is_empty(), "the eval corpus must not be empty");
                for path in entries {
                    let scenario: serde_json::Value = serde_json::from_str(
                        &fs::read_to_string(&path).expect("readable scenario"),
                    )
                    .expect("valid scenario JSON");
                    by_category
                        .entry(scenario["category"].as_str().expect("category").to_string())
                        .or_insert_with(Vec::new)
                        .push((
                            scenario["id"].as_str().expect("id").to_string(),
                            scenario["prompt"].as_str().expect("prompt").to_string(),
                            scenario["rfc_use_case"].as_u64(),
                        ));
                }
                by_category
            })
            .clone()
    }

    #[test]
    fn all_refusal_corpus_prompts_are_refused_with_the_right_rfc_class() {
        let corpus = corpus();
        let packs = builtin_packs();
        let refusals = corpus.get("refusal").expect("refusal scenarios committed");
        assert_eq!(refusals.len(), 4, "one refusal scenario per RFC class");
        for (id, prompt, rfc) in refusals {
            let refusal = screen(prompt, &packs[0])
                .unwrap_or_else(|| panic!("{id} must be refused: {prompt:?}"));
            assert_eq!(
                u64::from(refusal.rfc_use_case),
                rfc.expect("refusal scenarios carry rfc_use_case"),
                "{id} refused under the wrong class: {}",
                refusal.reason
            );
            assert!(
                refusal.reason.contains("out of platform scope")
                    && refusal.reason.contains("trust feature"),
                "{id}'s written reason must quote the RFC rationale: {}",
                refusal.reason
            );
        }
    }

    #[test]
    fn every_legitimate_corpus_prompt_is_admitted() {
        let corpus = corpus();
        let packs = builtin_packs();
        for (category, scenarios) in &corpus {
            if category == "refusal" {
                continue;
            }
            for (id, prompt, _) in scenarios {
                if let Some(refusal) = screen(prompt, &packs[0]) {
                    panic!(
                        "false positive: {category}/{id} ({prompt:?}) was refused as \
                         RFC {} — tune the rule groups",
                        refusal.rfc_use_case
                    );
                }
            }
        }
    }

    /// The single hot words that appear in legitimate clinical language must
    /// never fire alone — the grouped rules exist exactly for this.
    #[test]
    fn hot_words_in_legitimate_shapes_do_not_fire() {
        let packs = builtin_packs();
        for prompt in [
            "flag rising pain scores and escalate to the surgeon's inbox",
            "a staff triage queue for our practice's inbound messages",
            "track each patient's diagnosis codes for billing",
            "sync intake summaries into the chart before the visit",
            "benchmark our clinic's own no-show rate month over month",
            "chest pain follow-up notes for my cardiology patients",
        ] {
            assert!(
                screen(prompt, &packs[0]).is_none(),
                "false positive on legitimate shape: {prompt:?}"
            );
        }
    }

    /// Paraphrases of the four refused shapes (not the committed corpus
    /// wording) must still be caught — the rules key on the shape, not the
    /// exact fixture sentence.
    #[test]
    fn paraphrased_out_of_scope_shapes_are_refused() {
        let packs = builtin_packs();
        for (prompt, rfc) in [
            ("a symptom checker that tells patients when to call 911", 15),
            (
                "triage patients' symptoms and route the urgent ones to the ER",
                15,
            ),
            ("AI that interprets home EKG readings and detects afib", 21),
            (
                "software to automatically diagnose skin lesions from photos",
                21,
            ),
            ("an ONC certified data exchange platform for the region", 10),
            (
                "an interoperability platform syncing every EHR in the county",
                10,
            ),
            (
                "risk-adjusted outcomes benchmarking across our health system",
                9,
            ),
        ] {
            let refusal = screen(prompt, &packs[0])
                .unwrap_or_else(|| panic!("paraphrase must be refused: {prompt:?}"));
            assert_eq!(refusal.rfc_use_case, rfc, "wrong class for {prompt:?}");
        }
    }
}
